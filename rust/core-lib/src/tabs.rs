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

//! The main container for core state.
//!
//! All events from the frontend or from plugins are handled here first.
//!
//! This file is called 'tabs' for historical reasons, and should probably
//! be renamed.

use std::collections::{BTreeMap, HashSet};
use std::cell::{Cell, RefCell};
use std::fmt;
use std::fs::File;
use std::io;
use std::mem;
use std::path::{Path, PathBuf};

use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};
use serde_json::Value;

use xi_rpc::{self, RpcPeer, RpcCtx, RemoteError};
use xi_rope::Rope;
use xi_trace::{self, trace_block};

use WeakXiCore;
use client::Client;
use config::{self, ConfigManager, ConfigDomain, ConfigDomainExternal, Table};
use editor::Editor;
use event_context::EventContext;
use file::FileManager;
use plugins::{PluginCatalog, PluginPid, Plugin, start_plugin_process};
use plugin_rpc::{PluginNotification, PluginRequest};
use rpc::{CoreNotification, CoreRequest, EditNotification, EditRequest,
          PluginNotification as CorePluginNotification};
use styles::ThemeStyleMap;
use view::View;
use width_cache::WidthCache;

#[cfg(feature = "notify")]
use watcher::{FileWatcher, WatchToken};
#[cfg(feature = "notify")]
use notify::DebouncedEvent;
#[cfg(feature = "notify")]
use std::ffi::OsStr;

/// ViewIds are the primary means of routing messages between
/// xi-core and a client view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViewId(pub(crate) usize);

/// BufferIds uniquely identify open buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord,
         Serialize, Deserialize, Hash)]
pub struct BufferId(pub(crate) usize);

pub type PluginId = ::plugins::PluginPid;

// old-style names; will be deprecated
pub type BufferIdentifier = BufferId;
pub type ViewIdentifier = ViewId;

/// Totally arbitrary; we reserve this space for `ViewId`s
pub(crate) const RENDER_VIEW_IDLE_MASK: usize = 1 << 25;

const NEW_VIEW_IDLE_TOKEN: usize = 1001;

/// xi_rpc idle Token for watcher related idle scheduling.
pub(crate) const WATCH_IDLE_TOKEN: usize = 1002;

#[cfg(feature = "notify")]
const CONFIG_EVENT_TOKEN: WatchToken = WatchToken(1);

/// Token for file-change events in open files
#[cfg(feature = "notify")]
pub const OPEN_FILE_EVENT_TOKEN: WatchToken = WatchToken(2);

#[allow(dead_code)]
pub struct CoreState {
    editors: BTreeMap<BufferId, RefCell<Editor>>,
    views: BTreeMap<ViewId, RefCell<View>>,
    file_manager: FileManager,
    /// A local pasteboard.
    kill_ring: RefCell<Rope>,
    /// Theme and style state.
    style_map: RefCell<ThemeStyleMap>,
    width_cache: RefCell<WidthCache>,
    /// User and platform specific settings
    config_manager: ConfigManager,
    /// A weak reference to the main state container, stashed so that
    /// it can be passed to plugins.
    self_ref: Option<WeakXiCore>,
    /// Views which need to have setup finished.
    pending_views: Vec<ViewId>,
    peer: Client,
    id_counter: Counter,
    plugins: PluginCatalog,
    // for the time being we auto-start all plugins we find on launch.
    running_plugins: Vec<Plugin>,
}

/// Initial setup and bookkeeping
impl CoreState {
    pub(crate) fn new(peer: &RpcPeer, config_dir: Option<PathBuf>,
                      extras_dir: Option<PathBuf>) -> Self
    {
        #[cfg(feature = "notify")]
        let mut watcher = FileWatcher::new(peer.clone());

        if let Some(p) = config_dir.as_ref() {
            if !p.exists() {
                if let Err(e) = config::init_config_dir(p) {
                    //TODO: report this error?
                    eprintln!("error initing file based configs: {:?}", e);
                }
            }

            #[cfg(feature = "notify")]
            watcher.watch_filtered(p, true, CONFIG_EVENT_TOKEN,
                                   |p| p.extension()
                                   .and_then(OsStr::to_str)
                                   .unwrap_or("") == "xiconfig" );
        }

        let config_manager = ConfigManager::new(config_dir, extras_dir);

        CoreState {
            views: BTreeMap::new(),
            editors: BTreeMap::new(),
            #[cfg(feature = "notify")]
            file_manager: FileManager::new(watcher),
            #[cfg(not(feature = "notify"))]
            file_manager: FileManager::new(),
            kill_ring: RefCell::new(Rope::from("")),
            style_map: RefCell::new(ThemeStyleMap::new()),
            width_cache: RefCell::new(WidthCache::new()),
            config_manager,
            self_ref: None,
            pending_views: Vec::new(),
            peer: Client::new(peer.clone()),
            id_counter: Counter::default(),
            plugins: PluginCatalog::default(),
            running_plugins: Vec::new(),
        }
    }

    fn next_view_id(&self) -> ViewId {
        ViewId(self.id_counter.next())
    }

    fn next_buffer_id(&self) -> BufferId {
        BufferId(self.id_counter.next())
    }

    fn next_plugin_id(&self) -> PluginId {
        PluginPid(self.id_counter.next())
    }

    pub(crate) fn finish_setup(&mut self, self_ref: WeakXiCore) {
        self.self_ref = Some(self_ref);

        if let Some(path) = self.config_manager.base_config_file_path() {
            self.load_file_based_config(&path);
        }

        // instead of having to do this here, config should just own
        // the plugin catalog and reload automatically
        let plugin_paths = self.config_manager.get_plugin_paths();
        self.plugins.reload_from_paths(&plugin_paths);
        let languages = self.plugins.make_languages_map();
        self.config_manager.set_langauges(languages);
        let theme_names = self.style_map.borrow().get_theme_names();
        self.peer.available_themes(theme_names);

        // FIXME: temporary: we just launch every plugin we find at startup
        for manifest in self.plugins.iter() {
            start_plugin_process(manifest.clone(),
                                 self.next_plugin_id(),
                                 self.self_ref.as_ref().unwrap().clone());
        }
    }

    /// Attempt to load a config file.
    fn load_file_based_config(&mut self, path: &Path) {
        let _t = trace_block("CoreState::load_config_file", &["core"]);
        if let Some(domain) = self.config_manager.domain_for_path(path) {
            match config::try_load_from_file(&path) {
                Ok(table) => self.set_config(domain, table, Some(path.to_owned())),
                Err(e) => self.peer.alert(e.to_string()),
            }
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
            .for_each(|mut ctx| ctx.config_changed(&self.config_manager))
    }
}

/// Handling client events
impl CoreState {

    /// Creates an `EventContext` for the provided `ViewId`. This context
    /// holds references to the `Editor` and `View` backing this `ViewId`,
    /// as well as to sibling views, plugins, and other state necessary
    /// for handling most events.
    pub(crate) fn make_context<'a>(&'a self, view_id: ViewId)
        -> Option<EventContext<'a>>
    {
        self.views.get(&view_id).map(|view| {
            let buffer_id = view.borrow().buffer_id;

            let editor = self.editors.get(&buffer_id).unwrap();
            let info = self.file_manager.get_info(buffer_id);
            let plugins = self.running_plugins.iter().collect::<Vec<_>>();

            EventContext {
                view,
                editor,
                info: info,
                siblings: Vec::new(),
                plugins: plugins,
                client: &self.peer,
                style_map: &self.style_map,
                width_cache: &self.width_cache,
                kill_ring: &self.kill_ring,
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

    pub(crate) fn client_notification(&mut self, cmd: CoreNotification) {
        use self::CoreNotification::*;
        use self::CorePluginNotification as PN;
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
            SaveTrace { destination, frontend_samples } =>
                self.save_trace(&destination, frontend_samples),
            Plugin(cmd) =>
                match cmd {
                    PN::Start { view_id, plugin_name } =>
                        self.do_start_plugin(view_id, &plugin_name),
                    PN::Stop { view_id, plugin_name } =>
                        self.do_stop_plugin(view_id, &plugin_name),
                    PN::PluginRpc { .. } => ()
                        //TODO: rethink custom plugin RPCs
                }
            TracingConfig { enabled } =>
                self.toggle_tracing(enabled),
            // handled at the top level
            ClientStarted { .. } => (),
        }
    }

    pub(crate) fn client_request(&mut self, cmd: CoreRequest)
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
            GetConfig { view_id } =>
                self.do_get_config(view_id).map(|c| json!(c)),
        }
    }

    fn do_edit(&mut self, view_id: ViewId, cmd: EditNotification) {
        if let Some(mut edit_ctx) = self.make_context(view_id) {
            edit_ctx.do_edit(cmd);
        }
    }

    fn do_edit_sync(&mut self, view_id: ViewId,
                    cmd: EditRequest) -> Result<Value, RemoteError> {
        if let Some(mut edit_ctx) = self.make_context(view_id) {
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
        let syntax = self.config_manager.language_for_path(path);
        let config = self.config_manager.get_buffer_config(syntax, buffer_id);
        let editor = Editor::with_text(rope, config);
        Ok(editor)
    }

    fn do_save<P>(&mut self, view_id: ViewId, path: P)
        where P: AsRef<Path>
    {
        let _t = trace_block("CoreState::do_save", &["core"]);
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

        let syntax = self.config_manager.language_for_path(path);
        let config = self.config_manager.get_buffer_config(syntax, buffer_id);
        //TODO: rework how config changes are handled if a path changes.
        //tldr; do the save first, then reload the config.

        let mut event_ctx = self.make_context(view_id).unwrap();
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

        self.iter_groups().for_each(|mut edit_ctx| {
            edit_ctx.with_editor(|ed, view, _| {
                ed.theme_changed(&self.style_map.borrow());
                view.set_dirty(ed.get_buffer());
            });
            edit_ctx.render_if_needed();
        });
    }

    // NOTE: this is coming in from a direct RPC; unlike `set_config`, missing
    // keys here are left in their current state (`set_config` clears missing keys)
    /// Updates the config for a given domain.
    fn do_modify_user_config(&mut self, domain: ConfigDomainExternal,
                             changes: Table) {
        // the client sends ViewId but we need BufferId so we do a dance
        let domain: ConfigDomain = match domain {
            ConfigDomainExternal::General => ConfigDomain::General,
            ConfigDomainExternal::Syntax(id) => ConfigDomain::Language(id),
            ConfigDomainExternal::Language(id) => ConfigDomain::Language(id),
            ConfigDomainExternal::UserOverride(view_id) => {
                 match self.views.get(&view_id) {
                     Some(v) => ConfigDomain::UserOverride(v.borrow().buffer_id),
                     None => return,
                }
            }
        };
        if let Err(e) = self.config_manager.update_user_config(domain, changes) {
            self.peer.alert(e.to_string());
        }
        self.after_config_change();
    }

    fn do_get_config(&self, view_id: ViewId) -> Result<Table, RemoteError> {
        let _t = trace_block("CoreState::get_config", &["core"]);
        self.make_context(view_id)
            .map(|mut ctx| ctx.with_editor(|ed, _, _| ed.get_config().to_table()))
            .ok_or(RemoteError::custom(404, format!("missing {}", view_id), None))
    }

    fn do_start_plugin(&mut self, _view_id: ViewId, plugin: &str) {
        if self.running_plugins.iter().any(|p| p.name == plugin) {
            eprintln!("plugin {} already running", plugin);
            return;
        }

        if let Some(manifest) = self.plugins.get_named(plugin) {
            //TODO: lots of races possible here, we need to keep track of
            //pending launches.
            start_plugin_process(manifest.clone(),
                                 self.next_plugin_id(),
                                 self.self_ref.as_ref().unwrap().clone());
        } else {
            eprintln!("no plugin found with name '{}'", plugin);
        }
    }

    fn do_stop_plugin(&mut self, _view_id: ViewId, plugin: &str) {
        if let Some(p) = self.running_plugins.iter()
            .position(|p| p.name == plugin)
            .map(|ix| self.running_plugins.remove(ix)) {
                //TODO: verify shutdown; kill if necessary
                p.shutdown();
                self.iter_groups().for_each(|mut cx| cx.plugin_stopped(&p));

            }
    }
}

/// Idle, tracing, and file event handling
impl CoreState {
    pub(crate) fn handle_idle(&mut self, token: usize) {
        match token {
            NEW_VIEW_IDLE_TOKEN => self.finalize_new_views(),
            WATCH_IDLE_TOKEN => self.handle_fs_events(),
            other if (other & RENDER_VIEW_IDLE_MASK) != 0 =>
                self.handle_render_timer(other ^ RENDER_VIEW_IDLE_MASK),
            other => panic!("unexpected idle token {}", other),
        };
    }

    fn finalize_new_views(&mut self) {
        let to_start = mem::replace(&mut self.pending_views, Vec::new());
        to_start.iter().for_each(|id| {
            let mut edit_ctx = self.make_context(*id).unwrap();
            edit_ctx.finish_init();
        });
    }

    fn handle_render_timer(&mut self, token: usize) {
        let id: ViewId = token.into();
        if let Some(mut ctx) = self.make_context(id) {
            ctx._finish_delayed_render();
        }
    }

    #[cfg(feature = "notify")]
    fn handle_fs_events(&mut self) {
        let _t = trace_block("CoreState::handle_fs_events", &["core"]);
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
    #[cfg(feature = "notify")]
    fn handle_open_file_fs_event(&mut self, event: DebouncedEvent) {
        use notify::DebouncedEvent::*;
        let path = match event {
            NoticeWrite(ref path) |
                Create(ref path) |
                Write(ref path) => path,
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
            Create(ref path) | Write(ref path) =>
                self.load_file_based_config(path),
            Remove(ref path) =>
                self.config_manager.remove_source(path),
            Rename(ref old, ref new) => {
                self.config_manager.remove_source(old);
                let should_load = self.config_manager.should_load_file(new);
                if should_load { self.load_file_based_config(new) }
            }
            _ => (),
        }
    }

    fn toggle_tracing(&self, enabled: bool) {
        self.running_plugins.iter()
            .for_each(|plugin| plugin.toggle_tracing(enabled))
    }

    fn save_trace<P>(&self, path: P, frontend_samples: Value)
        where P: AsRef<Path>,
    {
        use xi_trace_dump::*;
        let mut all_traces = xi_trace::samples_cloned_unsorted();
        if let Ok(mut traces) = chrome_trace::decode(frontend_samples) {
            all_traces.append(&mut traces);
        }

        for plugin in self.running_plugins.iter() {
            match plugin.collect_trace() {
                Ok(json) => {
                    let mut trace = chrome_trace::decode(json).unwrap();
                    all_traces.append(&mut trace);
                }
                Err(e) => eprintln!("trace error {:?}", e),
            }
        }

        all_traces.sort_unstable();

        let mut trace_file = match File::create(path.as_ref()) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("error saving trace {:?}", e);
                return;
            }
        };

        if let Err(e) = chrome_trace::serialize(&all_traces, &mut trace_file) {
            eprintln!("error saving trace {:?}", e);
        }
    }
}

/// plugin event handling
impl CoreState {
    /// Called from a plugin's thread after trying to start the plugin.
    pub(crate) fn plugin_connect(&mut self,
                                  plugin: Result<Plugin, io::Error>) {
        match plugin {
            Ok(plugin) => {
                let init_info = self.iter_groups()
                    .map(|mut ctx| ctx.plugin_info())
                    .collect::<Vec<_>>();
                plugin.initialize(init_info);
                self.iter_groups().for_each(|mut cx| cx.plugin_started(&plugin));
                self.running_plugins.push(plugin);
            }
            Err(e) => eprintln!("failed to start plugin {:?}", e),
        }
    }

    /// Handles the response to a sync update sent to a plugin.
    pub(crate) fn plugin_update(&mut self, _plugin_id: PluginId, view_id: ViewId,
                                 response: Result<Value, xi_rpc::Error>) {

        if let Some(mut edit_ctx) = self.make_context(view_id) {
            edit_ctx.do_plugin_update(response);
        }
    }

    pub(crate) fn plugin_notification(&mut self, _ctx: &RpcCtx,
                                       view_id: ViewId, plugin_id: PluginId,
                                       cmd: PluginNotification) {
        if let Some(mut edit_ctx) = self.make_context(view_id) {
            edit_ctx.do_plugin_cmd(plugin_id, cmd)
        }
    }

    pub(crate) fn plugin_request(&mut self, _ctx: &RpcCtx, view_id: ViewId,
                                  plugin_id: PluginId, cmd: PluginRequest
                                  ) -> Result<Value, RemoteError>
    {
        if let Some(mut edit_ctx) = self.make_context(view_id) {
            Ok(edit_ctx.do_plugin_cmd_sync(plugin_id, cmd))
        } else {
            Err(RemoteError::custom(404, "missing view", None))
        }
    }
}

/// test helpers
impl CoreState {
    pub fn _test_open_editors(&self) -> Vec<BufferId> {
        self.editors.keys().cloned().collect()
    }

    pub fn _test_open_views(&self) -> Vec<ViewId> {
        self.views.keys().cloned().collect()
    }
}

pub mod test_helpers {
    use super::{ViewId, BufferId};

    pub fn new_view_id(id: usize) -> ViewId {
        ViewId(id)
    }

    pub fn new_buffer_id(id: usize) -> BufferId {
        BufferId(id)
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
        self.0.set(n + 1);
        n + 1
    }
}

impl<'a> From<&'a str> for ViewId {
    fn from(s: &'a str) -> Self {
        let ord = s.trim_left_matches("view-id-");
        let ident = usize::from_str_radix(ord, 10)
            .expect("ViewId parsing should never fail");
        ViewId(ident)
    }
}

impl From<String> for ViewId {
    fn from(s: String) -> Self {
        s.as_str().into()
    }
}

// these two only exist so that we can use ViewIds as idle tokens
impl From<usize> for ViewId {
    fn from(src: usize) -> ViewId {
        ViewId(src)
    }
}

impl From<ViewId> for usize {
    fn from(src: ViewId) -> usize {
        src.0
    }
}

impl fmt::Display for ViewId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "view-id-{}", self.0)
    }
}

impl Serialize for ViewId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where S: Serializer
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ViewId
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where D: Deserializer<'de>
    {
        let s = String::deserialize(deserializer)?;
        Ok(s.into())
    }
}

impl fmt::Display for BufferId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "buffer-id-{}", self.0)
    }
}

impl BufferId {
    pub fn new(val: usize) -> Self {
        BufferId(val)
    }
}
