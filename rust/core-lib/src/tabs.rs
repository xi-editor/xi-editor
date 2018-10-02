// Copyright 2016 Google Inc. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! A container for all the documents being edited. Also functions as main dispatch for RPC.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt;
use std::io::{self, Read};
use std::path::{PathBuf, Path};
use std::fs::File;
use std::sync::{Arc, Mutex, MutexGuard, Weak, mpsc};
use std::time::SystemTime;

use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};
use serde_json::{self, Value};
#[cfg(feature = "notify")]
use notify::DebouncedEvent;

use xi_rope::rope::Rope;
use xi_rpc::{self, RpcCtx, RemoteError};
use xi_trace::{self, trace_block};

use editor::Editor;

use rpc;
use config;
#[cfg(feature = "notify")]
use watcher::{FileWatcher, WatchToken};
use styles::{Style, ThemeStyleMap};
use MainPeer;

use syntax::SyntaxDefinition;
use config::{ConfigManager, ConfigDomain, Table};
use plugins::{self, PluginManagerRef, Command};
use plugins::rpc::{PluginUpdate, ClientPluginInfo};

#[cfg(feature="ledger")]
use apps_ledger_services_public::{Ledger_Proxy};

/// Token for config-related file change events
#[cfg(feature = "notify")]
const CONFIG_EVENT_TOKEN: WatchToken = WatchToken(1);
/// Token for file-change events in open files
const OPEN_FILE_EVENT_TOKEN: WatchToken = WatchToken(2);

const NEW_VIEW_IDLE_TOKEN: usize = 1001;

/// xi_rpc idle Token for watcher related idle scheduling.
pub const WATCH_IDLE_TOKEN: usize = 1002;

/// ViewIdentifiers are the primary means of routing messages between xi-core and a client view.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViewIdentifier(usize);

/// BufferIdentifiers uniquely identify open buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash)]
pub struct BufferIdentifier(usize);

/// Tracks open buffers, and relationships between buffers and views.
pub struct BufferContainer {
    /// associates open file paths to buffers
    open_files: BTreeMap<PathBuf, BufferIdentifier>,
    /// maps buffer identifiers to editor instances
    editors: BTreeMap<BufferIdentifier, Editor>,
    /// maps view identifiers to buffer identifiers. All actions originate in a view;
    /// this lets us route messages correctly when multiple views share a buffer.
    views: BTreeMap<ViewIdentifier, BufferIdentifier>,
}

/// Wrapper around `Arc<Mutex<`[`BufferContainer`][BufferContainer]`>>`,
/// for more ergonomic synchronization.
///
/// `BufferContainerRef` provides a thread-safe API for accessing and modifying the
/// [`BufferContainer`][BufferContainer]. All methods on `BufferContainerRef`
/// are thread safe. For finer grained actions, the underlying container can be accessed
/// via `BufferContainer::lock`.
///
/// [BufferContainer]: struct.BufferContainer.html
pub struct BufferContainerRef(Arc<Mutex<BufferContainer>>);

/// Wrapper around a `Weak<Mutex<BufferContainer>>`
///
/// `WeakBufferContaienrRef` provides a more ergonomic way of storing a `Weak`
/// reference to a [`BufferContainer`][BufferContainer].
///
/// [BufferContainer]: struct.BufferContainer.html
pub struct WeakBufferContainerRef(Weak<Mutex<BufferContainer>>);


/// A container for all open documents.
///
/// `Documents` is effectively the apex of the xi's model graph. It keeps references
/// to all active `Editor ` instances (through a `BufferContainerRef` instance),
/// and handles dispatch of RPC methods between client views and `Editor`
/// instances, as well as between `Editor` instances and Plugins.
pub struct Documents {
    /// keeps track of buffer/view state.
    buffers: BufferContainerRef,
    id_counter: usize,
    kill_ring: Arc<Mutex<Rope>>,
    style_map: Arc<Mutex<ThemeStyleMap>>,
    plugins: PluginManagerRef,
    config_manager: ConfigManager,
    #[cfg(feature = "notify")]
    file_watcher: Option<FileWatcher>,
    /// A tx channel used to propagate plugin updates from all `Editor`s.
    update_channel: mpsc::Sender<(ViewIdentifier, PluginUpdate, usize)>,
    /// A queue of closures to be executed on the next idle runloop pass.
    idle_queue: Vec<Box<IdleProc>>,
    #[allow(dead_code)]
    sync_repo: Option<SyncRepo>,
}

#[derive(Clone)]
/// A container for state shared between `Editor` instances.
pub struct DocumentCtx {
    kill_ring: Arc<Mutex<Rope>>,
    rpc_peer: MainPeer,
    style_map: Arc<Mutex<ThemeStyleMap>>,
    update_channel: mpsc::Sender<(ViewIdentifier, PluginUpdate, usize)>
}

#[derive(Serialize, Deserialize)]
/// A request for measuring the widths of strings all of the same style (a request
/// from core to front-end).
pub struct WidthReq {
    pub id: usize,
    pub strings: Vec<String>,
}

/// A trait for closure types which are callable with a `Documents` instance.
trait IdleProc: Send {
    fn call(self: Box<Self>, docs: &mut Documents);
}


impl<F: Send + FnOnce(&mut Documents)> IdleProc for F {
    fn call(self: Box<F>, docs: &mut Documents) {
        (*self)(docs)
    }
}

impl BufferContainer {
    /// Returns a reference to the `Editor` instance owning `view_id`'s view.
    pub fn editor_for_view(&self, view_id: ViewIdentifier) -> Option<&Editor> {
        match self.views.get(&view_id) {
            Some(id) => self.editors.get(id),
            None => {
                eprintln!("no buffer_id for view {}", view_id);
                None
            }
        }
    }

    /// Returns a mutable reference to the `Editor` instance owning
    /// `view_id`'s view.
    pub fn editor_for_view_mut(&mut self, view_id: ViewIdentifier)
                               -> Option<&mut Editor> {
        match self.views.get(&view_id) {
            Some(id) => self.editors.get_mut(id),
            None => {
                eprintln!("no buffer_id for view {}", view_id);
                None
            }
        }
    }

    /// Returns an iterator over all active `Editor`s.
    pub fn iter_editors<'a>(&'a self) -> Box<Iterator<Item=&'a Editor> + 'a> {
        Box::new(self.editors.values())
    }

    /// Returns a mutable iterator over all active `Editor`s.
    pub fn iter_editors_mut<'a>(&'a mut self) -> Box<Iterator<Item=&'a mut Editor> + 'a> {
        Box::new(self.editors.values_mut())
    }

    /// Returns a mutable reference to the `Editor` instance with `id`
    pub fn editor_for_buffer_mut(&mut self, id: &BufferIdentifier)
                                 -> Option<&mut Editor> {
        self.editors.get_mut(id)
    }
}

impl BufferContainerRef {
    pub fn new() -> Self {
        BufferContainerRef(Arc::new(Mutex::new(
                    BufferContainer {
                        open_files: BTreeMap::new(),
                        editors: BTreeMap::new(),
                        views: BTreeMap::new(),
                    })))
    }

    /// Returns a handle to the inner `MutexGuard`.
    pub fn lock(&self) -> MutexGuard<BufferContainer> {
        self.0.lock().unwrap()
    }

    /// Creates a new `WeakBufferContainerRef`.
    pub fn to_weak(&self) -> WeakBufferContainerRef {
        let weak_inner = Arc::downgrade(&self.0);
        WeakBufferContainerRef(weak_inner)
    }

    /// Returns `true` if `file_path` is already open, else `false`.
    pub fn has_open_file<P: AsRef<Path>>(&self, file_path: P) -> bool {
        self.lock().open_files.contains_key(file_path.as_ref())
    }

    /// if `file_path` is currently open, returns the `BufferIdentifier` of
    /// the corresponding `Editor`.
    pub fn editor_for_path<P>(&self, path: P) -> Option<BufferIdentifier>
        where P: AsRef<Path>,
    {
        self.lock().open_files.get(path.as_ref()).cloned()
    }

    /// Returns a copy of the BufferIdentifier associated with a given view.
    pub fn buffer_for_view(&self, view_id: ViewIdentifier) -> Option<BufferIdentifier> {
        self.lock().views.get(&view_id).map(|id| id.to_owned())
    }

    /// Adds a new editor, associating it with the provided identifiers.
    pub fn add_editor(&self, view_id: ViewIdentifier, buffer_id: BufferIdentifier,
                      editor: Editor) {
        let mut inner = self.lock();
        inner.views.insert(view_id, buffer_id);
        inner.editors.insert(buffer_id, editor);
    }

    /// Registers `file_path` as an open file, associated with `view_id`'s buffer.
    ///
    /// If an existing path is already associated with this buffer, it is removed.
    pub fn set_path<P: AsRef<Path>>(&self, file_path: P, view_id: ViewIdentifier) {
        let file_path = file_path.as_ref();
        let mut inner = self.lock();
        let buffer_id = inner.views[&view_id].to_owned();
        let prev_path = inner.editor_for_view(view_id).unwrap()
            .get_path().map(Path::to_owned);
        if let Some(prev_path) = prev_path {
            if prev_path != file_path {
                inner.open_files.remove(&prev_path);
            }
        }
        inner.open_files.insert(file_path.to_owned(), buffer_id);
        inner.editor_for_view_mut(view_id).unwrap()._set_path(file_path);
    }

    /// Closes the view with identifier `view_id`.
    ///
    /// If this is the last view open onto the underlying buffer, also cleans up
    /// the `Editor` instance.
    pub fn close_view(&self, view_id: ViewIdentifier) {
        let mut inner = self.lock();
        let buffer_id = inner.views.remove(&view_id);
        let ed = buffer_id.and_then(|id| inner.editors.remove(&id));
        let ed = match ed {
            Some(ed) => ed,
            None => return,
        };

        if let Some(path) = ed.get_path() {
            inner.open_files.remove(path);
        }
    }
}

impl WeakBufferContainerRef {
    /// Upgrades the weak reference to an Arc, if possible.
    ///
    /// Returns `None` if the inner value has been deallocated.
    pub fn upgrade(&self) -> Option<BufferContainerRef> {
        match self.0.upgrade() {
            Some(inner) => Some(BufferContainerRef(inner)),
            None => None
        }
    }
}

impl Clone for BufferContainerRef {
    fn clone(&self) -> Self {
        BufferContainerRef(self.0.clone())
    }
}

impl Documents {
    pub fn new() -> Documents {
        let buffers = BufferContainerRef::new();
        let config_manager = ConfigManager::default();
        let plugin_manager = PluginManagerRef::new(buffers.clone());
        let (update_tx, update_rx) = mpsc::channel();

        plugins::start_update_thread(update_rx, &plugin_manager);

        Documents {
            buffers,
            id_counter: 0,
            kill_ring: Arc::new(Mutex::new(Rope::from(""))),
            style_map: Arc::new(Mutex::new(ThemeStyleMap::new())),
            plugins: plugin_manager,
            config_manager,
            #[cfg(feature = "notify")]
            file_watcher: None,
            update_channel: update_tx,
            idle_queue: Vec::new(),
            sync_repo: None,
        }
    }

    #[doc(hidden)]
    pub fn _get_buffers(&self) -> BufferContainerRef {
        self.buffers.clone()
    }

    fn new_tab_ctx(&self, peer: &MainPeer) -> DocumentCtx {
        DocumentCtx {
            kill_ring: self.kill_ring.clone(),
            rpc_peer: peer.clone(),
            style_map: self.style_map.clone(),
            update_channel: self.update_channel.clone(),
        }
    }

    fn next_view_id(&mut self) -> ViewIdentifier {
        self.id_counter += 1;
        format!("view-id-{}", self.id_counter).into()
    }

    fn next_buffer_id(&mut self) -> BufferIdentifier {
        self.id_counter += 1;
        BufferIdentifier(self.id_counter)
    }

    fn toggle_tracing(&self, enabled: bool) {
        if enabled {
            eprintln!("Enabling tracing in core");
            xi_trace::enable_tracing();
        } else {
            eprintln!("Disabling tracing in core");
            xi_trace::disable_tracing();
        }

        self.plugins.toggle_tracing(enabled);
    }

    fn save_trace<P: AsRef<Path>>(&self, destination: P, frontend_samples: &Value) {
        use xi_trace_dump::*;

        let mut frontend_trace = chrome_trace::decode(frontend_samples)
            .unwrap_or(Vec::with_capacity(0));

        let mut plugin_traces = self.plugins.collect_trace()
            .into_iter()
            .map(|json| chrome_trace::decode(&json).unwrap())
            .collect::<Vec<_>>();

        eprintln!("Saving trace to {:?}", destination.as_ref());

        let trace_file_result = File::create(destination.as_ref());
        if trace_file_result.is_err() {
            eprintln!("Failed to create file: {:?}", trace_file_result.unwrap_err());
            return;
        }
        let mut trace_file = trace_file_result.unwrap();

        let mut samples = xi_trace::samples_cloned_unsorted();
        for mut traces in &mut plugin_traces {
            samples.append(&mut traces);
        }

        samples.append(&mut frontend_trace);
        samples.sort_unstable();
        let serialize_result = chrome_trace::serialize(
            &samples, chrome_trace::OutputFormat::JsonArray,
            &mut trace_file);
        if serialize_result.is_err() {
            eprintln!("Failed to serialize samples: {:?}", serialize_result.unwrap_err());
            return;
        }
    }

    pub fn handle_notification(&mut self, cmd: rpc::CoreNotification,
                               rpc_ctx: &RpcCtx) {
        use rpc::CoreNotification::*;
        match cmd {
            ClientStarted { config_dir, client_extras_dir } =>
                self.do_client_init(rpc_ctx.get_peer(), config_dir,
                                    client_extras_dir),
            SetTheme { theme_name } =>
                self.do_set_theme(rpc_ctx.get_peer(), &theme_name),
            Save { view_id, file_path } =>
                self.do_save(rpc_ctx.get_peer(), view_id, file_path),
            CloseView { view_id } => self.do_close_view(view_id),
            Edit(rpc::EditCommand { view_id, cmd }) => {
                self.buffers.lock().editor_for_view_mut(view_id)
                    .map(|ed| ed.handle_notification(view_id, cmd));
                }
            Plugin(cmd) => self.do_plugin_cmd(cmd),
            ModifyUserConfig { domain, changes } =>
                self.do_modify_user_config(rpc_ctx.get_peer(), domain, changes),
            TracingConfig {enabled} => self.toggle_tracing(enabled),
            SaveTrace { destination, frontend_samples } => self.save_trace(&destination, &frontend_samples),
        }
    }

    pub fn handle_request(&mut self, cmd: rpc::CoreRequest,
                          rpc_ctx: &RpcCtx) -> Result<Value, RemoteError> {
        use rpc::CoreRequest::*;
        match cmd {
            NewView { file_path } => {
                let result = self.do_new_view(rpc_ctx.get_peer(), file_path);
                // schedule idle handler after creating views; this is used to
                // send cursors for empty views, and to initialize plugins.
                rpc_ctx.schedule_idle(NEW_VIEW_IDLE_TOKEN);
                Ok(result)
            }
            Edit(rpc::EditCommand { view_id, cmd }) => {
                let result = self.buffers.lock().editor_for_view_mut(view_id)
                    .map(|ed| ed.handle_request(view_id, cmd));
                match result {
                    None => {
                        let msg = format!("No editor for view_id: {}", view_id);
                        Err(RemoteError::custom(2, msg, None))
                    }
                    Some(result) => result,
                }
            },
            GetConfig { view_id } => {
                self.do_get_config(view_id)
                .map(|v| v.into())
                .map_err(|_| {
                    let msg = format!("No editor for view_id: {}", view_id);
                    RemoteError::custom(2, msg, None)
                })
            },
        }
    }

    /// Creates a new view and associates it with a buffer.
    ///
    /// This function always creates a new view and associates it with a buffer
    /// (which we access through an `Editor` instance). This buffer may be existing,
    /// or it may be created.
    ///
    /// A `new_view` request is handled differently depending on the `file_path`
    /// argument, and on application state. If `file_path` is given and a buffer
    /// associated with that file is already open, we create a new view into the
    /// existing buffer. If `file_path` is given and that file _isn't_ open,
    /// we load that file into a new buffer. If `file_path` is not given,
    /// we create a new empty buffer.
    fn do_new_view(&mut self, rpc_peer: &MainPeer, file_path: Option<String>) -> Value {
        // three code paths: new buffer, open file, and new view into existing buffer
        let view_id = self.next_view_id();
        if let Some(file_path) = file_path.map(PathBuf::from) {
            // TODO: here, we should eventually be adding views to the existing editor.
            // for the time being, we just create a new empty view.
            if self.buffers.has_open_file(&file_path) {
                let buffer_id = self.next_buffer_id();
                self.new_empty_view(rpc_peer, view_id, buffer_id);
            } else {
                // not open: create new buffer_id and open file
                let buffer_id = self.next_buffer_id();
                self.new_view_with_file(rpc_peer, view_id, buffer_id, &file_path);
            }
        } else {
            // file_path was nil: create a new empty buffer.
            let buffer_id = self.next_buffer_id();
            self.new_empty_view(rpc_peer, view_id, buffer_id);
        }

        // closure to handle post-creation work on next idle runloop
        let init_info = self.buffers.lock().editor_for_view(view_id)
            .unwrap().plugin_init_info();

        let on_idle = Box::new(move |self_ref: &mut Documents| {
            self_ref.plugins.document_new(view_id, &init_info);
            {
                let mut editors = self_ref.buffers.lock();
                editors.editor_for_view(view_id)
                    .map(|ed| ed.send_config_init());
                for editor in editors.iter_editors_mut() {
                    editor.render();
                }
            }
        });
        self.idle_queue.push(on_idle);
        json!(view_id)
    }

    fn do_close_view(&mut self, view_id: ViewIdentifier) {
        self.plugins.document_close(view_id);
        self.buffers.close_view(view_id);
    }

    fn new_empty_view(&mut self, rpc_peer: &MainPeer, view_id: ViewIdentifier,
                      buffer_id: BufferIdentifier) {
        let editor = Editor::new(self.new_tab_ctx(rpc_peer),
                                 self.config_manager.default_buffer_config(),
                                 buffer_id, view_id);
        self.add_editor(view_id, buffer_id, editor, None);
    }

    fn new_view_with_file(&mut self, rpc_peer: &MainPeer, view_id: ViewIdentifier,
                          buffer_id: BufferIdentifier, path: &Path) {
        match self.read_file(&path) {
            Ok(contents) => {
                let syntax = SyntaxDefinition::new(path.to_str());
                let config = self.config_manager.get_buffer_config(syntax, view_id);
                let ed = Editor::with_text(self.new_tab_ctx(rpc_peer), config,
                                           buffer_id, view_id, contents);
                self.add_editor(view_id, buffer_id, ed, Some(path));
            }
            Err(err) => {
                let ed = Editor::new(self.new_tab_ctx(rpc_peer),
                                     self.config_manager.default_buffer_config(),
                                     buffer_id, view_id);
                if path.exists() {
                    // if this is a read error of an actual file, we don't set path
                    // TODO: we should be reporting errors to the client
                    eprintln!("unable to read file: {}, error: {:?}", buffer_id, err);
                    self.add_editor(view_id, buffer_id, ed, None);
                } else {
                    // if a path that doesn't exist, create a new empty buffer + set path
                    self.add_editor(view_id, buffer_id, ed, Some(path));
                }
            }
        }
    }

    /// Adds a new editor, associating it with the provided identifiers.
    ///
    /// This is called once each time a new editor is created.
    fn add_editor(&mut self, view_id: ViewIdentifier, buffer_id: BufferIdentifier,
                  mut editor: Editor, path: Option<&Path>) {
        self.initialize_sync(&mut editor, path, buffer_id);
        self.buffers.add_editor(view_id, buffer_id, editor);
        if let Some(path) = path {
            self.buffers.set_path(path, view_id);
            self.add_watch_path(path);
        }
    }

    #[cfg(not(feature = "ledger"))]
    fn initialize_sync(&mut self, _editor: &mut Editor, _path_opt: Option<&Path>, _buffer_id: BufferIdentifier) {
        // not implemented yet on OSs other than Fuchsia
    }

    fn read_file<P: AsRef<Path>>(&self, path: P) -> io::Result<String> {
        let mut f = File::open(path)?;
        let mut s = String::new();
        f.read_to_string(&mut s)?;
        Ok(s)
    }

    fn do_save<P>(&mut self, peer: &MainPeer, view_id: ViewIdentifier, file_path: P)
        where P: AsRef<Path>
    {
        let _t = trace_block("Documents::do_save", &["core"]);
        let file_path = file_path.as_ref();
        let prev_syntax = self.buffers.lock().editor_for_view(view_id)
            .unwrap().get_syntax().to_owned();
        let prev_path = self.buffers.lock().editor_for_view(view_id)
            .and_then(|ed| ed.get_path().map(PathBuf::from));
        let new_syntax = SyntaxDefinition::new(file_path.to_str());

        // notify of syntax change before notify of file_save
        //FIXME: this doesn't tell us if the syntax _will_ change, for instance
        //if syntax was a user selection. (we don't handle this case right now)
        let mut is_new_file_path = true;
        if let Some(ref prev_path) = prev_path {
            if prev_path != file_path {
                self.remove_watch_path(prev_path);
            } else {
                is_new_file_path = false;
                // if we're already open at this path, check file hasn't changed
                if self.buffers.lock().editor_for_view(view_id)
                    .unwrap().get_file_has_changed() {
                        let err_msg = format!("File {:?} has changed on disk. \
                        Please save as something else. \
                        I'm sorry if this is annoying.", file_path);
                        peer.send_rpc_notification("alert", &json!({"msg": err_msg}));
                        return
                }
            }
        }

        let save_result = self.buffers.lock()
            .editor_for_view_mut(view_id).unwrap().do_save(file_path);
        match save_result {
            Ok(()) => {
                self.buffers.set_path(file_path, view_id);

                if is_new_file_path {
                    self.add_watch_path(file_path);
                }

                let init_info = self.buffers.lock().editor_for_view(view_id)
                    .unwrap().plugin_init_info();

                if prev_syntax != new_syntax {
                    self.plugins.document_syntax_changed(view_id, init_info);
                    let new_config = self.config_manager
                        .get_buffer_config(new_syntax, view_id);
                    self.buffers.lock().editor_for_view_mut(view_id)
                        .unwrap().set_config(new_config);
                }
                self.plugins.document_did_save(view_id, file_path);
            },
            Err(e) => peer.send_rpc_notification("alert",
                &json!({ "msg": e, })),
        }
    }

    /// Handles a plugin related command from a client
    fn do_plugin_cmd(&mut self, cmd: rpc::PluginNotification) {
        use rpc::PluginNotification::*;
        let _t = trace_block("Documents::do_plugin_cmd", &["core"]);
        match cmd {
            Start { view_id, plugin_name } => {
                //TODO: report this error to client?
                let info = self.buffers.lock().editor_for_view(view_id)
                    .map(|ed| ed.plugin_init_info());
                if let Some(info) = info {
                    let _ = self.plugins.start_plugin(view_id, &info, &plugin_name);
                }
            }
            Stop { view_id, plugin_name } => {
                eprintln!("stop plugin rpc {}", plugin_name);
                self.plugins.stop_plugin(view_id, &plugin_name);
            }
            PluginRpc  { view_id, receiver, rpc } => {
                assert!(rpc.params_ref().is_object(), "params must be an object");
                assert!(!rpc.is_request(), "client->plugin rpc is notification only");
                self.plugins.dispatch_command(view_id, &receiver,
                                              &rpc.method, &rpc.params);
            }
        }
    }

    fn do_client_init(&mut self, rpc_peer: &MainPeer, config_dir: Option<PathBuf>,
                      client_extras_dir: Option<PathBuf>) {
        let _t = trace_block("Documents::do_client_init", &["core"]);
        // we would like to set this in self::new but we need the peer
        #[cfg(feature = "notify")]
        { self.file_watcher = Some(FileWatcher::new(rpc_peer.clone())); }

        if let Some(ref d) = config_dir {
            self.config_manager.set_config_dir(&d);
            if let Err(e) = self.init_file_based_configs(d, rpc_peer) {
                eprintln!("Error reading config dir: {:?}", e);
            }
        }

        if let Some(ref d) = client_extras_dir {
            //TODO: test setting this when config_dir.is_none()
            self.config_manager.set_extras_dir(d);
        }

        let params = {
            let style_map = self.style_map.lock().unwrap();
            json!({
                "themes": style_map.get_theme_names()
            })
        };

        let plugin_paths = self.config_manager.plugin_search_path();
        self.plugins.set_plugin_search_path(plugin_paths);
        rpc_peer.send_rpc_notification("available_themes", &params);
    }

    /// Handle a client set theme RPC
    fn do_set_theme(&self, rpc_peer: &MainPeer, theme_name: &str) {
        let _t = trace_block("Documents::set_theme", &["core"]);
        let success = self.style_map.lock().unwrap()
            .set_theme(&theme_name).is_ok();
        if success {
            let params = {
                let style_map = self.style_map.lock().unwrap();
                json!({
                    "name": style_map.get_theme_name(),
                    "theme": style_map.get_theme_settings(),
                })
            };
            rpc_peer.send_rpc_notification("theme_changed", &params);

            let mut buffers = self.buffers.lock();
            for ed in buffers.iter_editors_mut() {
                ed.theme_changed();
            }
        } else {
            eprintln!("no theme named {}", theme_name);
        }
    }

    fn do_get_config(&self, view_id: ViewIdentifier) -> Result<Table, RemoteError> {
        let _t = trace_block("Documents::get_config", &["core"]);
        let view_config = self.buffers.lock().editor_for_view(view_id)
            .map(|ed| ed.get_config().to_table());
        view_config.ok_or(
            RemoteError::custom(2, &format!("No buffer for view {}", view_id), None))
    }

    fn add_watch_path(&mut self, path: &Path) {
        #[cfg(feature = "notify")]
        self.file_watcher.as_mut()
            .unwrap()
            .watch(path, false, OPEN_FILE_EVENT_TOKEN);
    }

    fn remove_watch_path(&mut self, path: &Path) {
        #[cfg(feature = "notify")]
        self.file_watcher.as_mut()
            .unwrap()
            .unwatch(path, OPEN_FILE_EVENT_TOKEN);
    }

    pub fn handle_idle(&mut self, ctx: &RpcCtx, token: usize) {
        match token {
            WATCH_IDLE_TOKEN => {
                #[cfg(feature = "notify")]
                self.handle_fs_events(ctx.get_peer())
            }
            NEW_VIEW_IDLE_TOKEN => {
                let _t = trace_block("Documents::new view idle", &["core"]);
                while let Some(f) = self.idle_queue.pop() {
                    f.call(self);
                }
            }
            _ => (),
        }
    }

    /// Process file system events, forwarding them to registrees.
    #[cfg(feature = "notify")]
    fn handle_fs_events(&mut self, peer: &MainPeer) {
        let _t = trace_block("Documents::handle_fs_events", &["core"]);
        let mut events = self.file_watcher.as_mut().unwrap().take_events();
        let mut config_changed = false;

        for (token, event) in events.drain(..) {
            match token {
                CONFIG_EVENT_TOKEN => {
                    //TODO: we should(?) be more efficient about this update,
                    // with config_manager returning whether it's necessary.
                    self.handle_config_fs_event(event, peer);
                    config_changed = true;
                }
                OPEN_FILE_EVENT_TOKEN => self.handle_open_file_fs_event(event),
                _ => eprintln!("unexpected fs event token {:?}", token),
            }
        }
        if config_changed {
            self.after_config_change();
        }
    }

    /// Handles a file system event related to a currently open file
    fn handle_open_file_fs_event(&mut self, event: DebouncedEvent) {
        use notify::DebouncedEvent::*;
        match event {
            NoticeWrite(ref path) |
            Create(ref path) |
            Write(ref path) => {
                let mod_time = get_file_mod_time(path);
                let id = self.buffers.editor_for_path(path);
                let mut inner = self.buffers.lock();
                let mut ed = match id.and_then(|x| inner.editor_for_buffer_mut(&x)) {
                    Some(ed) => ed,
                    None => return,
                };

                //TODO: currently we only use the file's modification time when
                // determining if a file has been changed by another process.
                // A more robust solution would also hash the file's contents.
                let has_changed_on_disk = ed.get_file_mod_time()
                    .map(|t| Some(t) != mod_time)
                    .unwrap_or(false);

                if has_changed_on_disk {
                    // if the buffer isn't dirty we can just reload the file
                    if ed.is_pristine() {
                        if let Ok(contents) = self.read_file(path) {
                            ed.reload(&contents);
                        }
                    } else {
                        ed._set_file_has_changed(true);
                    }
                }
            }
            other => eprintln!("Event in open file {:?}", other),
        }
    }

    /// Handles a config related file system event.
    #[cfg(feature = "notify")]
    fn handle_config_fs_event(&mut self, event: DebouncedEvent, peer: &MainPeer) {
        use self::DebouncedEvent::*;
        match event {
            Create(ref path) | Write(ref path) => {
                self.load_file_based_config(peer, path)
            }
            Remove(ref path) => self.config_manager.remove_source(path),
            Rename(ref old, ref new) => {
                self.config_manager.remove_source(old);
                let should_load = self.config_manager.should_load_file(new);
                if should_load { self.load_file_based_config(peer, new) }
            }
            _ => (),
        }
    }

    /// Checks for existence of config dir, loading config files and registering
    /// for file system events if the directory exists and can be read.
    fn init_file_based_configs(&mut self, config_dir: &Path,
                               peer: &MainPeer) -> io::Result<()> {
        let _t = trace_block("Documents::init_file_config", &["core"]);
        if !config_dir.exists() {
            config::init_config_dir(config_dir)?;
        }
        let config_files = config::iter_config_files(config_dir)?;
        config_files.for_each(|p| self.load_file_based_config(peer, &p));

        #[cfg(feature = "notify")]
        self.file_watcher.as_mut().unwrap()
            .watch_filtered(config_dir,
                            true,
                            CONFIG_EVENT_TOKEN,
                            |p| p.extension()
                                    .and_then(OsStr::to_str)
                                    .unwrap_or("") == "xiconfig" );
        Ok(())
    }

    /// Attempt to load a config file.
    fn load_file_based_config(&mut self, peer: &MainPeer, path: &Path) {
        match config::try_load_from_file(&path) {
            Ok((d, t)) => self.set_config(peer, d, t, Some(path.to_owned())),
            Err(e) => {
                let err_msg = format!("{}", &e);
                peer.send_rpc_notification("alert", &json!({"msg": err_msg}));
            }
        }
    }

    /// Sets (overwriting) the config for a given domain.
    fn set_config<P>(&mut self, peer: &MainPeer, domain: ConfigDomain,
                     table: Table, path: P)
        where P: Into<Option<PathBuf>>
    {
        if let Err(e) = self.config_manager.set_user_config(domain, table, path) {
            let err_msg = format!("{}", &e);
            peer.send_rpc_notification("alert", &json!({"msg": err_msg}));
        }
    }

    // NOTE: this is coming in from a direct RPC; unlike `set_config`, missing
    // keys here are left in their current state (`set_config` clears missing keys)
    /// Updates the config for a given domain.
    fn do_modify_user_config(&mut self, peer: &MainPeer, domain: ConfigDomain,
                             changes: Table) {
        if let Err(e) = self.config_manager.update_user_config(domain, changes) {
            let err_msg = format!("{}", &e);
            peer.send_rpc_notification("alert", &json!({"msg": err_msg}));
        }
        self.after_config_change();
    }


    /// Notify editors/views/plugins of config changes.
    fn after_config_change(&self) {
        let mut to_notify = Vec::new();
        {
            let mut editors = self.buffers.lock();
            for ed in editors.iter_editors_mut() {
                let syntax = ed.get_syntax().to_owned();
                let identifier = ed.get_main_view_id();
                let new_config = self.config_manager.get_buffer_config(syntax,
                                                                       identifier);
                if let Some(changes) = ed.set_config(new_config) {
                    to_notify.push((identifier, changes));
                }
            }
        }
        // update plugins after releasing the lock
        for (view_id, changes) in to_notify.drain(..) {
            self.plugins.document_config_changed(view_id, &changes);
        }
    }
}

#[cfg(feature = "ledger")]
impl Drop for Documents {
    fn drop(&mut self) {
        use std::mem;
        if let Some(repo) = mem::replace(&mut self.sync_repo, None) {
            repo.tx.send(SyncMsg::Stop).unwrap();
            repo.updater_handle.join().unwrap();
        }
    }
}

impl DocumentCtx {
    pub fn update_view(&self, view_id: ViewIdentifier, update: &Value) {
        self.rpc_peer.send_rpc_notification("update",
            &json!({
                "view_id": view_id,
                "update": update,
            }));
    }

    pub fn scroll_to(&self, view_id: ViewIdentifier, line: usize, col: usize) {
        self.rpc_peer.send_rpc_notification("scroll_to",
            &json!({
                "view_id": view_id,
                "line": line,
                "col": col,
            }));
    }

    pub fn config_changed(&self, view_id: &ViewIdentifier, changes: &Table) {
        self.rpc_peer.send_rpc_notification("config_changed",
                                            &json!({
                                                "view_id": view_id,
                                                "changes": changes,
                                            }));
    }

    /// Notify the client that a plugin ha started.
    pub fn plugin_started(&self, view_id: ViewIdentifier, plugin: &str) {
        self.rpc_peer.send_rpc_notification("plugin_started",
                                            &json!({
                                                "view_id": view_id,
                                                "plugin": plugin,
                                            }));
    }

    /// Notify the client that a plugin ha stopped.
    ///
    /// `code` is not currently used.
    pub fn plugin_stopped(&self, view_id: ViewIdentifier, plugin: &str, code: i32) {
        self.rpc_peer.send_rpc_notification("plugin_stopped",
                                            &json!({
                                                "view_id": view_id,
                                                "plugin": plugin,
                                                "code": code,
                                            }));
    }

    /// Notify the client of the available plugins.
    pub fn available_plugins(&self, view_id: ViewIdentifier,
                             plugins: &[ClientPluginInfo]) {
        self.rpc_peer.send_rpc_notification("available_plugins",
                                            &json!({
                                                "view_id": view_id,
                                                "plugins": plugins }));
    }

    pub fn update_cmds(&self, view_id: ViewIdentifier,
                       plugin: &str, cmds: &[Command]) {
        self.rpc_peer.send_rpc_notification("update_cmds",
                                            &json!({
                                                "view_id": view_id,
                                                "plugin": plugin,
                                                "cmds": cmds,
                                            }));
    }

    pub fn alert(&self, msg: &str) {
        self.rpc_peer.send_rpc_notification("alert",
            &json!({
                "msg": msg,
            }));
    }

    pub fn get_kill_ring(&self) -> Rope {
        self.kill_ring.lock().unwrap().clone()
    }

    pub fn set_kill_ring(&self, val: Rope) {
        let mut kill_ring = self.kill_ring.lock().unwrap();
        *kill_ring = val;
    }

    pub fn get_style_map(&self) -> &Arc<Mutex<ThemeStyleMap>> {
        &self.style_map
    }

    // Get the index for a given style. If the style is not in the existing
    // style map, then issues a def_style request to the front end. Intended
    // to be reasonably efficient, but ideally callers would do their own
    // indexing.
    pub fn get_style_id(&self, style: &Style) -> usize {
        let mut style_map = self.style_map.lock().unwrap();
        if let Some(ix) = style_map.lookup(style) {
            return ix;
        }
        let ix = style_map.add(style);
        let style = style_map.merge_with_default(style);
        self.rpc_peer.send_rpc_notification("def_style", &style.to_json(ix));
        ix
    }

    /// Notify plugins of an update
    pub fn update_plugins(&self, view_id: ViewIdentifier,
                          update: PluginUpdate, undo_group: usize) {
        self.update_channel.send((view_id, update, undo_group)).unwrap();
    }

    /// Ask front-end to measure widths of strings.
    pub fn measure_width(&self, reqs: &[WidthReq]) -> Result<Vec<Vec<f64>>, xi_rpc::Error> {
        let req_json = serde_json::to_value(reqs).expect("failed to serialize width req");
        let resp = self.rpc_peer.send_rpc_request("measure_width", &req_json)?;
        Ok(serde_json::from_value(resp).expect("failed to deserialize width response"))
    }
}

impl<'a> From<&'a str> for ViewIdentifier {
    fn from(s: &'a str) -> Self {
        let ord = s.trim_left_matches("view-id-");
        let ident = usize::from_str_radix(ord, 10)
            .expect("ViewIdentifier parsing should never fail");
        ViewIdentifier(ident)
    }
}

impl From<String> for ViewIdentifier {
    fn from(s: String) -> Self {
        s.as_str().into()
    }
}

// these two only exist so that we can use ViewIdentifiers as idle tokens
impl From<usize> for ViewIdentifier {
    fn from(src: usize) -> ViewIdentifier {
        ViewIdentifier(src)
    }
}

impl From<ViewIdentifier> for usize {
    fn from(src: ViewIdentifier) -> usize {
        src.0
    }
}

impl fmt::Display for ViewIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "view-id-{}", self.0)
    }
}

impl Serialize for ViewIdentifier {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where S: Serializer
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ViewIdentifier
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where D: Deserializer<'de>
    {
        let s = String::deserialize(deserializer)?;
        Ok(s.into())
    }
}

impl fmt::Display for BufferIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "buffer-id-{}", self.0)
    }
}

impl BufferIdentifier {
    pub fn new(val: usize) -> Self {
        BufferIdentifier(val)
    }
}

// =============== Fuchsia-specific synchronization plumbing
// We can't move this elsewhere since it requires access to private fields

#[cfg(not(feature = "ledger"))]
pub struct SyncRepo;

#[cfg(feature = "ledger")]
use std::sync::mpsc::{channel, Sender};
#[cfg(feature = "ledger")]
use std::thread;
#[cfg(feature = "ledger")]
use fuchsia::sync::{SyncStore, SyncMsg, SyncUpdater, start_conflict_resolver_factory};

#[cfg(feature = "ledger")]
pub struct SyncRepo {
    ledger: Ledger_Proxy,
    tx: Sender<SyncMsg>,
    updater_handle: thread::JoinHandle<()>,
    session_id: (u64,u32),
}

#[cfg(feature = "ledger")]
impl Documents {
    pub fn setup_ledger(&mut self, mut ledger: Ledger_Proxy, session_id: (u64,u32)) {
        let key = vec![0];
        start_conflict_resolver_factory(&mut ledger, key);

        let (tx, rx) = channel();
        let updater = SyncUpdater::new(self.buffers.clone(), rx);
        let updater_handle = thread::spawn(move|| updater.work().unwrap() );

        self.sync_repo = Some(SyncRepo { ledger, tx, updater_handle, session_id });
    }

    fn initialize_sync(&mut self, editor: &mut Editor, path_opt: Option<&Path>, buffer_id: &BufferIdentifier) {
        use apps_ledger_services_public::*;
        use fuchsia::ledger::{ledger_crash_callback, gen_page_id};

        if let (Some(path), Some(repo)) = (path_opt, self.sync_repo.as_mut()) {
            // TODO this will panic when loading a file with initial contents.
            // We haven't figured out what that even means in a multi-device
            // context so it's not clear we can do anything better.
            editor.set_session_id(repo.session_id);
            // create the sync ID based on the path
            // TODO: maybe make sync-id orthogonal to path
            let path_str = path.to_string_lossy();
            let path_bytes: &[u8] = path_str.as_bytes();
            let sync_id = gen_page_id(path_bytes);
            // get the page
            let (page, page_request) = Page_new_pair();
            repo.ledger.get_page(Some(sync_id.clone()), page_request).with(ledger_crash_callback);
            // create the SyncStore
            let sync_store = SyncStore::new(page, vec![0], repo.tx.clone(), buffer_id.clone());
            // set the SyncStore for the Editor
            editor.set_sync_store(sync_store);
        }
    }
}


/// Returns the modification timestamp for the file at a given path,
/// if present.
pub fn get_file_mod_time<P>(path: P) -> Option<SystemTime>
where P: AsRef<Path>
{
    File::open(path)
        .and_then(|f| f.metadata())
        .and_then(|meta| meta.modified())
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use xi_rpc::{RpcLoop};
    use std::env;
    use std::fs::File;
    use serde_json;

    // a bit of gymnastics to let us instantiate an Editor instance
    fn mock_doc_ctx(tempfile: &str) -> DocumentCtx {
        let mut dir = env::temp_dir();
        dir.push(tempfile);
        let f = File::create(dir).unwrap();

        let mock_loop = RpcLoop::new(f);
        let mock_peer = mock_loop.get_raw_peer();
        let (update_tx, _) = mpsc::channel();

        DocumentCtx {
            kill_ring: Arc::new(Mutex::new(Rope::from(""))),
            rpc_peer: Box::new(mock_peer.clone()),
            style_map: Arc::new(Mutex::new(ThemeStyleMap::new())),
            update_channel: update_tx,
        }
    }

    #[test]
    fn test_save_as() {
        let container_ref = BufferContainerRef::new();
        let config = ConfigManager::default().default_buffer_config();
        assert!(!container_ref.has_open_file("a fake file, for sure"));
        let view_id_1 = ViewIdentifier(1);
        let buf_id_1 = BufferIdentifier(1);
        let path_1 = PathBuf::from("a_path");
        let path_2 = PathBuf::from("a_different_path");
        let editor = Editor::new(mock_doc_ctx(&view_id_1.to_string()),
                                 config.clone(),
                                 buf_id_1, view_id_1);
        container_ref.add_editor(view_id_1, buf_id_1, editor);
        assert_eq!(container_ref.lock().editors.len(), 1);

        // set path (as if on save)
        container_ref.set_path(&path_1, view_id_1);
        assert_eq!(container_ref.has_open_file(&path_1), true);
        assert_eq!(
            container_ref.lock().editor_for_view(view_id_1).unwrap().get_path(),
            Some(path_1.as_ref()));

        // then save somewhere else:
        container_ref.set_path(&path_2, view_id_1);
        assert_eq!(container_ref.lock().editors.len(), 1);
        assert_eq!(container_ref.has_open_file(&path_1), false);
        assert_eq!(container_ref.has_open_file(&path_2), true);
        assert_eq!(
            container_ref.lock().editor_for_view(view_id_1).unwrap().get_path(),
            Some(path_2.as_ref()));

        // reopen the original file:
        let view_id_2 = ViewIdentifier(2);
        let buf_id_2 = BufferIdentifier(2);
        let editor = Editor::new(mock_doc_ctx(&view_id_2.to_string()),
                                 config.clone(), buf_id_2, view_id_2);
        container_ref.add_editor(view_id_2, buf_id_2, editor);
        container_ref.set_path(&path_1, view_id_2);
        assert_eq!(container_ref.lock().editors.len(), 2);
        assert_eq!(container_ref.has_open_file(&path_1), true);
        assert_eq!(container_ref.has_open_file(&path_2), true);

        container_ref.close_view(view_id_1);
        assert_eq!(container_ref.lock().editors.len(), 1);
        assert_eq!(container_ref.has_open_file(&path_2), false);
        assert_eq!(container_ref.has_open_file(&path_1), true);

        container_ref.close_view(view_id_2);
        assert_eq!(container_ref.has_open_file(&path_2), false);
        assert_eq!(container_ref.lock().editors.len(), 0);
    }

    #[test]
    fn test_id_serde() {
        // check to see that struct with single string member serializes as string
        let view_id = ViewIdentifier(8);
        let as_val = serde_json::to_value(view_id).unwrap();
        assert_eq!(as_val.to_string(), "\"view-id-8\"");
    }

    #[test]
    fn test_struct_serde() {
        #[derive(Serialize, Deserialize)]
        struct TestStruct {
            name: String,
            view: ViewIdentifier,
            flag: u64,
        }
        let json = r#"
        {"name": "victor",
         "view": "view-id-6",
         "flag": 42
        }"#;

        let result: TestStruct = serde_json::from_str(json).unwrap();
        assert_eq!(result.view, ViewIdentifier(6));
    }
}
