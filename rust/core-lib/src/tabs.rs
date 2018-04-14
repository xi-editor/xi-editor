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

//! The main container for core state. This file is called 'tabs' for legacy
//! reasons.

use std::collections::{BTreeMap, HashSet};
use std::cell::{Cell, RefCell};
use std::ffi::OsStr;
use std::fmt;
use std::io;
use std::mem;
use std::path::{Path, PathBuf};

use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};
use serde_json::Value;

use xi_rpc::{RpcPeer, RpcCtx, RemoteError, Error as RpcError};
use xi_rope::{Rope};
use xi_trace::trace_block;

use WeakXiCore;
use client::Client;
use config::{self, ConfigManager, ConfigDomain, Table};
use editing::EventContext;
use editor::Editor;
use file::FileManager;
use plugins::{PluginCatalog, PluginPid, Plugin, start_plugin_process};
use plugin_rpc::{PluginNotification, PluginRequest};
use rpc::{CoreNotification, CoreRequest, EditNotification, EditRequest};
use styles::ThemeStyleMap;
use syntax::SyntaxDefinition;
use view::View;

#[cfg(feature = "notify")]
use watcher::{FileWatcher, WatchToken};
#[cfg(feature = "notify")]
use notify::DebouncedEvent;

/// ViewIdentifiers are the primary means of routing messages between
/// xi-core and a client view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViewIdentifier(pub (crate) usize);

/// BufferIdentifiers uniquely identify open buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord,
         Serialize, Deserialize, Hash)]
pub struct BufferIdentifier(pub (crate) usize);

// new-style names; old names will be deprecated
pub type BufferId = BufferIdentifier;
pub type ViewId = ViewIdentifier;
pub type PluginId = ::plugins::PluginPid;

const NEW_VIEW_IDLE_TOKEN: usize = 1001;

/// xi_rpc idle Token for watcher related idle scheduling.
pub const WATCH_IDLE_TOKEN: usize = 1002;

#[cfg(feature = "notify")]
const CONFIG_EVENT_TOKEN: WatchToken = WatchToken(1);

/// Token for file-change events in open files
pub const OPEN_FILE_EVENT_TOKEN: WatchToken = WatchToken(2);

#[allow(dead_code)]
pub struct CoreState {
    editors: BTreeMap<BufferId, RefCell<Editor>>,
    views: BTreeMap<ViewId, RefCell<View>>,
    file_manager: FileManager,
    /// A local pasteboard.
    kill_ring: Rope,
    /// Theme and style state.
    style_map: RefCell<ThemeStyleMap>,
    /// User and platform specific settings
    config_manager: ConfigManager,
    /// A weak reference to the main state container, stashed so that
    /// it can be passed to plugins.
    self_ref: Option<WeakXiCore>,
    /// Views which need to have setup finished.
    pending_views: Vec<ViewId>,
    peer: Client,
    id_counter: Counter,
    // only support one plugin during refactor
    plugins: PluginCatalog,
    syntect: Option<Plugin>,
}

#[allow(dead_code)]
/// Initial setup and bookkeeping
impl CoreState {
    pub (crate) fn new(peer: &RpcPeer) -> Self {
        let watcher = FileWatcher::new(peer.clone());
        CoreState {
            views: BTreeMap::new(),
            editors: BTreeMap::new(),
            file_manager: FileManager::new(watcher),
            kill_ring: Rope::from(""),
            style_map: RefCell::new(ThemeStyleMap::new()),
            config_manager: ConfigManager::default(),
            self_ref: None,
            pending_views: Vec::new(),
            peer: Client::new(peer.clone()),
            id_counter: Counter::default(),
            plugins: PluginCatalog::new(&[]),
            syntect: None,
        }
    }

    fn next_view_id(&self) -> ViewId {
        ViewIdentifier(self.id_counter.next())
    }

    fn next_buffer_id(&self) -> BufferId {
        BufferIdentifier(self.id_counter.next())
    }

    fn next_plugin_id(&self) -> PluginId {
        PluginPid(self.id_counter.next())
    }

    pub (crate) fn finish_setup(&mut self, self_ref: WeakXiCore,
                                config_dir: Option<PathBuf>,
                                extras_dir: Option<PathBuf>) {

        self.self_ref = Some(self_ref);
        if let Some(ref path) = extras_dir {
            self.config_manager.set_extras_dir(path);
        }

        if let Some(ref path) = config_dir {
            self.config_manager.set_config_dir(path);
            //TODO: report this error
            let _ = self.init_file_based_configs(&path);
        }

        let plugin_paths = self.config_manager.plugin_search_path();
        self.plugins = PluginCatalog::from_paths(plugin_paths);

        let theme_names = self.style_map.borrow().get_theme_names();
        self.peer.available_themes(theme_names);

        // just during refactor, we manually start syntect at launch

        if let Some(manifest) = self.plugins.get_named("syntect") {
            start_plugin_process(manifest.clone(),
                                 self.next_plugin_id(),
                                 self.self_ref.as_ref().unwrap().clone());
        }
    }

    /// Checks for existence of config dir, loading config files and registering
    /// for file system events if the directory exists and can be read.
    fn init_file_based_configs(&mut self, config_dir: &Path) -> io::Result<()> {
        //TODO: we don't do this at setup because we previously didn't
        //know our config path at init time. we do now, so this can happen
        //at init time.
        let _t = trace_block("Documents::init_file_config", &["core"]);
        if !config_dir.exists() {
            config::init_config_dir(config_dir)?;
        }
        let config_files = config::iter_config_files(config_dir)?;
        config_files.for_each(|p| self.load_file_based_config(&p));

        #[cfg(feature = "notify")]
        self.file_manager.watcher()
            .watch_filtered(config_dir, true, CONFIG_EVENT_TOKEN,
                            |p| p.extension()
                            .and_then(OsStr::to_str)
                            .unwrap_or("") == "xiconfig" );
        Ok(())
    }

    /// Attempt to load a config file.
    fn load_file_based_config(&mut self, path: &Path) {
        match config::try_load_from_file(&path) {
            Ok((d, t)) => self.set_config(d, t, Some(path.to_owned())),
            Err(e) => self.peer.alert(e.to_string()),
        }
    }

    /// Sets (overwriting) the config for a given domain.
    fn set_config<P>(&mut self, domain: ConfigDomain, table: Table, path: P)
        where P: Into<Option<PathBuf>>
    {
        if let Err(e) = self.config_manager.set_user_config(domain, table, path) {
            self.peer.alert(format!("{}", &e));
        }
    }
    /// Notify editors/views/plugins of config changes.
    fn after_config_change(&self) {
        self.iter_groups()
            .for_each(|ctx| ctx.config_changed(&self.config_manager))
    }
}

/// Handling client events
impl CoreState {

    /// Creates an `EventContext` for the provided `ViewId`. This context
    /// holds references to the `Editor` and `View` backing this `ViewId`,
    /// as well as to sibling views, plugins, and other state necessary
    /// for handling most events.
    pub (crate) fn make_context<'a>(&'a self, view_id: ViewId)
        -> Option<EventContext<'a>>
    {
        self.views.get(&view_id).map(|view| {
            let buffer_id = view.borrow().buffer_id;

            let buffer = self.editors.get(&buffer_id).unwrap();
            let info = self.file_manager.get_info(buffer_id);

            let mut plugins = Vec::new();
            if let Some(syntect) = self.syntect.as_ref() {
                plugins.push(syntect);
            }
            EventContext {
                view,
                buffer,
                info: info,
                siblings: Vec::new(),
                plugins: plugins,
                client: &self.peer,
                style_map: &self.style_map,
                weak_core: self.self_ref.as_ref().unwrap(),
            }
        })
    }

    /// Produces an iterator over all event contexts, with each view appearing
    /// exactly once.
    fn iter_groups<'a>(&'a self) -> Iter<'a, Box<Iterator<Item=&ViewId> + 'a>>
    {
        Iter {
            views: Box::new(self.views.keys()),
            seen: HashSet::new(),
            inner: self,
        }
    }

    pub (crate) fn client_notification(&mut self, cmd: CoreNotification) {
        use self::CoreNotification::*;
        match cmd {
            Edit(::rpc::EditCommand { view_id, cmd }) =>
                self.do_edit(view_id, cmd),
            Save { view_id, file_path } =>
                self.do_save(view_id, file_path),
            CloseView { view_id } =>
                self.do_close_view(view_id),
            ModifyUserConfig { domain, changes } =>
                self.do_modify_user_config(domain, changes),
            SetTheme { theme_name } =>
                self.do_set_theme(&theme_name),
            SaveTrace { .. } => (),
            //TODO: restore me
                //self.save_trace(&destination, &frontend_samples),
            Plugin(..) => (),
                //self.do_plugin_cmd(cmd),
            // these two are handled at the top level
            ClientStarted { .. } => (),
            TracingConfig { .. } => (),
        }
    }

    pub (crate) fn client_request(&mut self, cmd: CoreRequest)
        -> Result<Value, RemoteError>
    {
        use self::CoreRequest::*;
        match cmd {
            //TODO: make file_path be an Option<PathBuf>
            //TODO: make this a notification
            NewView { file_path } =>
                self.do_new_view(file_path.map(PathBuf::from)),
            Edit(::rpc::EditCommand { view_id, cmd }) =>
                self.do_edit_sync(view_id, cmd),
            //TODO: why is this a request?? make a notification?
            GetConfig { view_id } => Ok(1.into()),
                //self.do_get_config(ctx, view_id),
        }
    }

    fn do_edit(&mut self, view_id: ViewId, cmd: EditNotification) {
        if let Some(edit_ctx) = self.make_context(view_id) {
            edit_ctx.do_edit(cmd);
        }
    }

    fn do_edit_sync(&mut self, view_id: ViewId,
                    cmd: EditRequest) -> Result<Value, RemoteError> {
        if let Some(edit_ctx) = self.make_context(view_id) {
            edit_ctx.do_edit_sync(cmd)
        } else {
            // TODO: some custom error tpye that can Into<RemoteError>
            Err(RemoteError::custom(404,
                                    format!("missing view {:?}", view_id),
                                    None))
        }
    }

    fn do_new_view(&mut self, path: Option<PathBuf>)
        -> Result<Value, RemoteError>
    {
        let view_id = self.next_view_id();
        let buffer_id = self.next_buffer_id();

        let editor = match path {
            Some(path) => self.new_with_file(&path, buffer_id)?,
            None => self.new_empty_buffer(),
        };

        let mut view = View::new(view_id, buffer_id);

        let wrap_width = editor.get_config().items.wrap_width;
        view.rewrap(editor.get_buffer(), wrap_width);
        view.set_dirty(editor.get_buffer());

        let editor = RefCell::new(editor);
        let view = RefCell::new(view);

        self.editors.insert(buffer_id, editor);
        self.views.insert(view_id, view);
        //NOTE: because this is a synchronous call, we have to return the
        //view_id before we can send any events to this view. We use mark the
        // viewa s pending and schedule the idle handler so that we can finish
        // setting up this view on the next runloop pass.
        self.pending_views.push(view_id);
        self.peer.schedule_idle(NEW_VIEW_IDLE_TOKEN);

        Ok(json!(view_id))
    }

    fn new_empty_buffer(&mut self) -> Editor {
        let config = self.config_manager.default_buffer_config();
        Editor::new(config)
    }

    fn new_with_file(&mut self, path: &Path, buffer_id: BufferId)
        -> Result<Editor, RemoteError>
    {
        let rope = self.file_manager.open(path, buffer_id)?;
        let syntax = SyntaxDefinition::new(path.to_str());
        let config = self.config_manager.get_buffer_config(syntax, buffer_id);
        let editor = Editor::with_text(rope, config);
        Ok(editor)
    }

    fn do_save<P>(&mut self, view_id: ViewId, path: P)
        where P: AsRef<Path>
    {
        let path = path.as_ref();
        let buffer_id = self.views.get(&view_id).map(|v| v.borrow().buffer_id);
        let buffer_id = match buffer_id {
            Some(id) => id,
            None => return,
        };

        let ed = self.editors.get(&buffer_id).unwrap();

        let result = self.file_manager.save(path, ed.borrow().get_buffer(),
                                            buffer_id);
        if let Err(e) = result {
            self.peer.alert(e.to_string());
            return;
        }

        // hacky, syntax defs per-se are going away soon
        let syntax = SyntaxDefinition::new(path.to_str());
        let config = self.config_manager.get_buffer_config(syntax, buffer_id);

        let event_ctx = self.make_context(view_id).unwrap();
        event_ctx.after_save(path, config);
    }

    fn do_close_view(&mut self, view_id: ViewId) {
        let close_buffer = self.make_context(view_id)
            .map(|ctx| ctx.close_view())
            .unwrap_or(true);

        let buffer_id = self.views.remove(&view_id)
            .map(|v| v.borrow().buffer_id);

        if let Some(buffer_id) = buffer_id {
            if close_buffer {
                self.editors.remove(&buffer_id);
                self.file_manager.close(buffer_id);
            }
        }
    }

    fn do_set_theme(&self, theme_name: &str) {
        if self.style_map.borrow_mut().set_theme(&theme_name).is_err() {
        //TODO: report error
            return;
        }
        {
            let style_map = self.style_map.borrow();
            self.peer.theme_changed(style_map.get_theme_name(),
                                    style_map.get_theme_settings());
        }

        self.iter_groups().for_each(|edit_ctx| {
            edit_ctx.with_editor(|ed, view| {
                ed.theme_changed(&self.style_map.borrow());
                view.set_dirty(ed.get_buffer());
            });
            edit_ctx.render();
        });
    }

    // NOTE: this is coming in from a direct RPC; unlike `set_config`, missing
    // keys here are left in their current state (`set_config` clears missing keys)
    /// Updates the config for a given domain.
    fn do_modify_user_config(&mut self, domain: ConfigDomain, changes: Table) {
        if let Err(e) = self.config_manager.update_user_config(domain, changes) {
            self.peer.alert(e.to_string());
        }
        self.after_config_change();
    }
}


/// Idle and file event handling
impl CoreState {
    pub (crate) fn handle_idle(&mut self, token: usize) {
        match token {
            NEW_VIEW_IDLE_TOKEN => self.finalize_new_views(),
            WATCH_IDLE_TOKEN => self.handle_fs_events(),
            _ => panic!("unexpected idle token {}", token),
        };
    }

    fn finalize_new_views(&mut self) {
        let to_start = mem::replace(&mut self.pending_views, Vec::new());
        to_start.iter().for_each(|id| {
            let edit_ctx = self.make_context(*id).unwrap();
            edit_ctx.finish_init();
        });
    }

    #[cfg(feature = "notify")]
    fn handle_fs_events(&mut self) {
        let _t = trace_block("Documents::handle_fs_events", &["core"]);
        let mut events = self.file_manager.watcher().take_events();
        let mut config_changed = false;

        for (token, event) in events.drain(..) {
            match token {
                OPEN_FILE_EVENT_TOKEN => self.handle_open_file_fs_event(event),
                CONFIG_EVENT_TOKEN => {
                    //TODO: we should(?) be more efficient about this update,
                    // with config_manager returning whether it's necessary.
                    self.handle_config_fs_event(event);
                    config_changed = true;
                }
                _ => eprintln!("unexpected fs event token {:?}", token),
            }
        }
        if config_changed {
            self.after_config_change();
        }
    }

    #[cfg(not(feature = "notify"))]
    fn handle_fs_events(&mut self) { }

    /// Handles a file system event related to a currently open file
    fn handle_open_file_fs_event(&mut self, event: DebouncedEvent) {
        use notify::DebouncedEvent::*;
        let path = match event {
            NoticeWrite(ref path @ _) |
                Create(ref path @ _) |
                Write(ref path @ _) => path,
            other => {
                eprintln!("Event in open file {:?}", other);
                return;
            }
        };

        let buffer_id = match self.file_manager.get_editor(path) {
            Some(id) => id,
            None => return,
        };

        let has_changes = self.file_manager.check_file(path, buffer_id);
        let is_pristine = self.editors.get(&buffer_id)
            .map(|ed| ed.borrow().is_pristine()).unwrap();
        //TODO: currently we only use the file's modification time when
        // determining if a file has been changed by another process.
        // A more robust solution would also hash the file's contents.

        if has_changes && is_pristine {
            if let Ok(text) = self.file_manager.open(path, buffer_id) {
                // this is ugly; we don't map buffer_id -> view_id anywhere
                // but we know we must have a view.
                let view_id = self.views.values()
                    .find(|v| v.borrow().buffer_id == buffer_id)
                    .map(|v| v.borrow().view_id)
                    .unwrap();
                self.make_context(view_id).unwrap().reload(text);
            }
        }
    }

    /// Handles a config related file system event.
    #[cfg(feature = "notify")]
    fn handle_config_fs_event(&mut self, event: DebouncedEvent) {
        use self::DebouncedEvent::*;
        match event {
            Create(ref path) | Write(ref path) => {
                self.load_file_based_config(path)
            }
            Remove(ref path) => self.config_manager.remove_source(path),
            Rename(ref old, ref new) => {
                self.config_manager.remove_source(old);
                let should_load = self.config_manager.should_load_file(new);
                if should_load { self.load_file_based_config(new) }
            }
            _ => (),
        }
    }
}

/// plugin event handling
impl CoreState {
    /// Called from a plugin's thread after trying to start the plugin.
    pub (crate) fn plugin_connect(&mut self,
                                  plugin: Result<Plugin, io::Error>) {
        match plugin {
            Ok(plugin) => {
                assert_eq!(&plugin.name, "syntect");
                let init_info = self.iter_groups()
                    .map(|ctx| ctx.plugin_info())
                    .collect::<Vec<_>>();
                plugin.initialize(init_info);
                self.syntect = Some(plugin);
                //TODO: notify views that plugin started
            }
            Err(e) => eprintln!("failed to start plugin {:?}", e),
        }
    }

    /// Handles the response to a sync update sent to a plugin.
    pub (crate) fn plugin_update(&mut self, _plugin_id: PluginId,
                                 view_id: ViewId, undo_group: usize,
                                 response: Result<Value, RpcError>) {

        if let Some(edit_ctx) = self.make_context(view_id) {
            edit_ctx.do_plugin_update(response, undo_group);
        }
    }

    pub (crate) fn plugin_notification(&mut self, _ctx: &RpcCtx,
                                       view_id: ViewId, plugin_id: PluginId,
                                       cmd: PluginNotification) {
        if let Some(edit_ctx) = self.make_context(view_id) {
            edit_ctx.do_plugin_cmd(plugin_id, cmd)
        }
    }

    pub (crate) fn plugin_request(&mut self, _ctx: &RpcCtx, view_id: ViewId,
                                  plugin_id: PluginId, cmd: PluginRequest
                                  ) -> Result<Value, RemoteError>
    {
        if let Some(edit_ctx) = self.make_context(view_id) {
            Ok(edit_ctx.do_plugin_cmd_sync(plugin_id, cmd))
        } else {
            Err(RemoteError::custom(404, "missing view", None))
        }
    }
}

/// A multi-view aware iterator over `EventContext`s. A view which appears
/// as a sibling will not appear again as a main view.
pub struct Iter<'a, I> {
    views: I,
    seen: HashSet<ViewId>,
    inner: &'a CoreState,
}

impl<'a, I> Iterator for Iter<'a, I> where I: Iterator<Item=&'a ViewId> {
    type Item = EventContext<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let &mut Iter { ref mut views, ref mut seen, ref inner } = self;
        loop {
            let next_view = match views.next() {
                None => return None,
                Some(v) if seen.contains(v) => continue,
                Some(v) => v,
            };
            let context = inner.make_context(*next_view).unwrap();
            context.siblings.iter().for_each(|sibl| {
                let _ = seen.insert(sibl.borrow().view_id);
            });
            return Some(context);
        }
    }
}

#[derive(Debug, Default)]
struct Counter(Cell<usize>);

impl Counter {
    fn next(&self) -> usize {
        let n = self.0.get();
        self.0.set(n+1);
        n
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
