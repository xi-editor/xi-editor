// Copyright 2016 The xi-editor Authors.
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

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs::File;
use std::io;
use std::mem;
use std::path::{Path, PathBuf};

use serde::de::{self, Deserialize, Deserializer, Unexpected};
use serde::ser::{Serialize, Serializer};
use serde_json::Value;

use xi_rope::Rope;
use xi_rpc::{self, ReadError, RemoteError, RpcCtx, RpcPeer};
use xi_trace::{self, trace_block};

use crate::client::Client;
use crate::config::{self, ConfigDomain, ConfigDomainExternal, ConfigManager, Table};
use crate::editor::Editor;
use crate::event_context::EventContext;
use crate::file::FileManager;
use crate::line_ending::LineEnding;
use crate::plugin_rpc::{PluginNotification, PluginRequest};
use crate::plugins::rpc::ClientPluginInfo;
use crate::plugins::{start_plugin_process, Plugin, PluginCatalog, PluginPid};
use crate::recorder::Recorder;
use crate::rpc::{
    CoreNotification, CoreRequest, EditNotification, EditRequest,
    PluginNotification as CorePluginNotification,
};
use crate::styles::{ThemeStyleMap, DEFAULT_THEME};
use crate::syntax::LanguageId;
use crate::view::View;
use crate::whitespace::Indentation;
use crate::width_cache::WidthCache;
use crate::WeakXiCore;

#[cfg(feature = "notify")]
use crate::watcher::{FileWatcher, WatchToken};
#[cfg(feature = "notify")]
use notify::Event;
#[cfg(feature = "notify")]
use std::ffi::OsStr;

/// ViewIds are the primary means of routing messages between
/// xi-core and a client view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViewId(pub(crate) usize);

/// BufferIds uniquely identify open buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash)]
pub struct BufferId(pub(crate) usize);

pub type PluginId = crate::plugins::PluginPid;

// old-style names; will be deprecated
pub type BufferIdentifier = BufferId;

/// Totally arbitrary; we reserve this space for `ViewId`s
pub(crate) const RENDER_VIEW_IDLE_MASK: usize = 1 << 25;
pub(crate) const REWRAP_VIEW_IDLE_MASK: usize = 1 << 26;
pub(crate) const FIND_VIEW_IDLE_MASK: usize = 1 << 27;

const NEW_VIEW_IDLE_TOKEN: usize = 1001;

/// xi_rpc idle Token for watcher related idle scheduling.
pub(crate) const WATCH_IDLE_TOKEN: usize = 1002;

#[cfg(feature = "notify")]
const CONFIG_EVENT_TOKEN: WatchToken = WatchToken(1);

/// Token for file-change events in open files
#[cfg(feature = "notify")]
pub const OPEN_FILE_EVENT_TOKEN: WatchToken = WatchToken(2);

#[cfg(feature = "notify")]
const THEME_FILE_EVENT_TOKEN: WatchToken = WatchToken(3);

#[cfg(feature = "notify")]
const PLUGIN_EVENT_TOKEN: WatchToken = WatchToken(4);

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
    /// Recorded editor actions
    recorder: RefCell<Recorder>,
    /// A weak reference to the main state container, stashed so that
    /// it can be passed to plugins.
    self_ref: Option<WeakXiCore>,
    /// Views which need to have setup finished.
    pending_views: Vec<(ViewId, Table)>,
    peer: Client,
    id_counter: Counter,
    plugins: PluginCatalog,
    // for the time being we auto-start all plugins we find on launch.
    running_plugins: Vec<Plugin>,
}

/// Initial setup and bookkeeping
impl CoreState {
    pub(crate) fn new(
        peer: &RpcPeer,
        config_dir: Option<PathBuf>,
        extras_dir: Option<PathBuf>,
    ) -> Self {
        #[cfg(feature = "notify")]
        let mut watcher = FileWatcher::new(peer.clone());

        if let Some(p) = config_dir.as_ref() {
            if !p.exists() {
                if let Err(e) = config::init_config_dir(p) {
                    //TODO: report this error?
                    error!("error initing file based configs: {:?}", e);
                }
            }

            #[cfg(feature = "notify")]
            watcher.watch_filtered(p, true, CONFIG_EVENT_TOKEN, |p| {
                p.extension().and_then(OsStr::to_str).unwrap_or("") == "xiconfig"
            });
        }

        let config_manager = ConfigManager::new(config_dir, extras_dir);

        let themes_dir = config_manager.get_themes_dir();
        if let Some(p) = themes_dir.as_ref() {
            #[cfg(feature = "notify")]
            watcher.watch_filtered(p, true, THEME_FILE_EVENT_TOKEN, |p| {
                p.extension().and_then(OsStr::to_str).unwrap_or("") == "tmTheme"
            });
        }

        let plugins_dir = config_manager.get_plugins_dir();
        if let Some(p) = plugins_dir.as_ref() {
            #[cfg(feature = "notify")]
            watcher.watch_filtered(p, true, PLUGIN_EVENT_TOKEN, |p| p.is_dir() || !p.exists());
        }

        CoreState {
            views: BTreeMap::new(),
            editors: BTreeMap::new(),
            #[cfg(feature = "notify")]
            file_manager: FileManager::new(watcher),
            #[cfg(not(feature = "notify"))]
            file_manager: FileManager::new(),
            kill_ring: RefCell::new(Rope::from("")),
            style_map: RefCell::new(ThemeStyleMap::new(themes_dir)),
            width_cache: RefCell::new(WidthCache::new()),
            config_manager,
            recorder: RefCell::new(Recorder::new()),
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

        // Load the custom theme files.
        self.style_map.borrow_mut().load_theme_dir();

        // instead of having to do this here, config should just own
        // the plugin catalog and reload automatically
        let plugin_paths = self.config_manager.get_plugin_paths();
        self.plugins.reload_from_paths(&plugin_paths);
        let languages = self.plugins.make_languages_map();
        let languages_ids = languages.iter().map(|l| l.name.clone()).collect::<Vec<_>>();
        self.peer.available_languages(languages_ids);
        self.config_manager.set_languages(languages);
        let theme_names = self.style_map.borrow().get_theme_names();
        self.peer.available_themes(theme_names);

        // FIXME: temporary: we just launch every plugin we find at startup
        for manifest in self.plugins.iter() {
            start_plugin_process(
                manifest.clone(),
                self.next_plugin_id(),
                self.self_ref.as_ref().unwrap().clone(),
            );
        }
    }

    /// Attempt to load a config file.
    fn load_file_based_config(&mut self, path: &Path) {
        let _t = trace_block("CoreState::load_config_file", &["core"]);
        if let Some(domain) = self.config_manager.domain_for_path(path) {
            match config::try_load_from_file(path) {
                Ok(table) => self.set_config(domain, table),
                Err(e) => self.peer.alert(e.to_string()),
            }
        } else {
            self.peer.alert(format!("Unexpected config file {:?}", path));
        }
    }

    /// Sets (overwriting) the config for a given domain.
    fn set_config(&mut self, domain: ConfigDomain, table: Table) {
        match self.config_manager.set_user_config(domain, table) {
            Err(e) => self.peer.alert(format!("{}", &e)),
            Ok(changes) => self.handle_config_changes(changes),
        }
    }

    /// Notify editors/views/plugins of config changes.
    fn handle_config_changes(&self, changes: Vec<(BufferId, Table)>) {
        for (id, table) in changes {
            let view_id = self
                .views
                .values()
                .find(|v| v.borrow().get_buffer_id() == id)
                .map(|v| v.borrow().get_view_id())
                .unwrap();

            self.make_context(view_id).unwrap().config_changed(&table)
        }
    }
}

/// Handling client events
impl CoreState {
    /// Creates an `EventContext` for the provided `ViewId`. This context
    /// holds references to the `Editor` and `View` backing this `ViewId`,
    /// as well as to sibling views, plugins, and other state necessary
    /// for handling most events.
    pub(crate) fn make_context(&self, view_id: ViewId) -> Option<EventContext> {
        self.views.get(&view_id).map(|view| {
            let buffer_id = view.borrow().get_buffer_id();

            let editor = &self.editors[&buffer_id];
            let info = self.file_manager.get_info(buffer_id);
            let plugins = self.running_plugins.iter().collect::<Vec<_>>();
            let config = self.config_manager.get_buffer_config(buffer_id);
            let language = self.config_manager.get_buffer_language(buffer_id);

            EventContext {
                view_id,
                buffer_id,
                view,
                editor,
                config: &config.items,
                recorder: &self.recorder,
                language,
                info,
                siblings: Vec::new(),
                plugins,
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
    fn iter_groups<'a>(&'a self) -> Iter<'a, Box<dyn Iterator<Item = &ViewId> + 'a>> {
        Iter { views: Box::new(self.views.keys()), seen: HashSet::new(), inner: self }
    }

    pub(crate) fn client_notification(&mut self, cmd: CoreNotification) {
        use self::CoreNotification::*;
        use self::CorePluginNotification as PN;
        match cmd {
            Edit(crate::rpc::EditCommand { view_id, cmd }) => self.do_edit(view_id, cmd),
            Save { view_id, file_path } => self.do_save(view_id, file_path),
            CloseView { view_id } => self.do_close_view(view_id),
            ModifyUserConfig { domain, changes } => self.do_modify_user_config(domain, changes),
            SetTheme { theme_name } => self.do_set_theme(&theme_name),
            SaveTrace { destination, frontend_samples } => {
                self.save_trace(&destination, frontend_samples)
            }
            Plugin(cmd) => match cmd {
                PN::Start { view_id, plugin_name } => self.do_start_plugin(view_id, &plugin_name),
                PN::Stop { view_id, plugin_name } => self.do_stop_plugin(view_id, &plugin_name),
                PN::PluginRpc { view_id, receiver, rpc } => {
                    self.do_plugin_rpc(view_id, &receiver, &rpc.method, &rpc.params)
                }
            },
            TracingConfig { enabled } => self.toggle_tracing(enabled),
            // handled at the top level
            ClientStarted { .. } => (),
            SetLanguage { view_id, language_id } => self.do_set_language(view_id, language_id),
        }
    }

    pub(crate) fn client_request(&mut self, cmd: CoreRequest) -> Result<Value, RemoteError> {
        use self::CoreRequest::*;
        match cmd {
            //TODO: make file_path be an Option<PathBuf>
            //TODO: make this a notification
            NewView { file_path } => self.do_new_view(file_path.map(PathBuf::from)),
            Edit(crate::rpc::EditCommand { view_id, cmd }) => self.do_edit_sync(view_id, cmd),
            //TODO: why is this a request?? make a notification?
            GetConfig { view_id } => self.do_get_config(view_id).map(|c| json!(c)),
            DebugGetContents { view_id } => self.do_get_contents(view_id).map(|c| json!(c)),
        }
    }

    fn do_edit(&mut self, view_id: ViewId, cmd: EditNotification) {
        if let Some(mut edit_ctx) = self.make_context(view_id) {
            edit_ctx.do_edit(cmd);
        }
    }

    fn do_edit_sync(&mut self, view_id: ViewId, cmd: EditRequest) -> Result<Value, RemoteError> {
        if let Some(mut edit_ctx) = self.make_context(view_id) {
            edit_ctx.do_edit_sync(cmd)
        } else {
            // TODO: some custom error tpye that can Into<RemoteError>
            Err(RemoteError::custom(404, format!("missing view {:?}", view_id), None))
        }
    }

    fn do_new_view(&mut self, path: Option<PathBuf>) -> Result<Value, RemoteError> {
        let view_id = self.next_view_id();
        let buffer_id = self.next_buffer_id();

        let rope = match path.as_ref() {
            Some(p) => self.file_manager.open(p, buffer_id)?,
            None => Rope::from(""),
        };

        let editor = RefCell::new(Editor::with_text(rope));
        let view = RefCell::new(View::new(view_id, buffer_id));

        self.editors.insert(buffer_id, editor);
        self.views.insert(view_id, view);

        let config = self.config_manager.add_buffer(buffer_id, path.as_deref());

        // NOTE: because this is a synchronous call, we have to initialize the
        // view and return the view_id before we can send any events to this
        // view. We call view_init(), mark the view as pending and schedule the
        // idle handler so that we can finish setting up this view on the next
        // runloop pass, in finalize_new_views.

        let mut edit_ctx = self.make_context(view_id).unwrap();
        edit_ctx.view_init();

        self.pending_views.push((view_id, config));
        self.peer.schedule_idle(NEW_VIEW_IDLE_TOKEN);

        Ok(json!(view_id))
    }

    fn do_save<P>(&mut self, view_id: ViewId, path: P)
    where
        P: AsRef<Path>,
    {
        let _t = trace_block("CoreState::do_save", &["core"]);
        let path = path.as_ref();
        let buffer_id = self.views.get(&view_id).map(|v| v.borrow().get_buffer_id());
        let buffer_id = match buffer_id {
            Some(id) => id,
            None => return,
        };

        let mut save_ctx = self.make_context(view_id).unwrap();
        let fin_text = save_ctx.text_for_save();

        if let Err(e) = self.file_manager.save(path, &fin_text, buffer_id) {
            let error_message = e.to_string();
            error!("File error: {:?}", error_message);
            self.peer.alert(error_message);
            return;
        }

        let changes = self.config_manager.update_buffer_path(buffer_id, path);
        let language = self.config_manager.get_buffer_language(buffer_id);

        self.make_context(view_id).unwrap().after_save(path);
        self.make_context(view_id).unwrap().language_changed(&language);

        // update the config _after_ sending save related events
        if let Some(changes) = changes {
            self.make_context(view_id).unwrap().config_changed(&changes);
        }
    }

    fn do_close_view(&mut self, view_id: ViewId) {
        let close_buffer = self.make_context(view_id).map(|ctx| ctx.close_view()).unwrap_or(true);

        let buffer_id = self.views.remove(&view_id).map(|v| v.borrow().get_buffer_id());

        if let Some(buffer_id) = buffer_id {
            if close_buffer {
                self.editors.remove(&buffer_id);
                self.file_manager.close(buffer_id);
                self.config_manager.remove_buffer(buffer_id);
            }
        }
    }

    fn do_set_theme(&self, theme_name: &str) {
        //Set only if requested theme is different from the
        //current one.
        if theme_name != self.style_map.borrow().get_theme_name() {
            if let Err(e) = self.style_map.borrow_mut().set_theme(theme_name) {
                error!("error setting theme: {:?}, {:?}", theme_name, e);
                return;
            }
        }
        self.notify_client_and_update_views();
    }

    fn notify_client_and_update_views(&self) {
        {
            let style_map = self.style_map.borrow();
            self.peer.theme_changed(style_map.get_theme_name(), style_map.get_theme_settings());
        }

        self.iter_groups().for_each(|mut edit_ctx| {
            edit_ctx.with_editor(|ed, view, _, _| {
                ed.theme_changed(&self.style_map.borrow());
                view.set_dirty(ed.get_buffer());
            });
            edit_ctx.render_if_needed();
        });
    }

    /// Updates the config for a given domain.
    fn do_modify_user_config(&mut self, domain: ConfigDomainExternal, changes: Table) {
        // the client sends ViewId but we need BufferId so we do a dance
        let domain: ConfigDomain = match domain {
            ConfigDomainExternal::General => ConfigDomain::General,
            ConfigDomainExternal::Syntax(id) => ConfigDomain::Language(id),
            ConfigDomainExternal::Language(id) => ConfigDomain::Language(id),
            ConfigDomainExternal::UserOverride(view_id) => match self.views.get(&view_id) {
                Some(v) => ConfigDomain::UserOverride(v.borrow().get_buffer_id()),
                None => return,
            },
        };
        let new_config = self.config_manager.table_for_update(domain.clone(), changes);
        self.set_config(domain, new_config);
    }

    fn do_get_config(&self, view_id: ViewId) -> Result<Table, RemoteError> {
        let _t = trace_block("CoreState::get_config", &["core"]);
        self.views
            .get(&view_id)
            .map(|v| v.borrow().get_buffer_id())
            .map(|id| self.config_manager.get_buffer_config(id).to_table())
            .ok_or(RemoteError::custom(404, format!("missing {}", view_id), None))
    }

    fn do_get_contents(&self, view_id: ViewId) -> Result<Rope, RemoteError> {
        self.make_context(view_id)
            .map(|ctx| ctx.editor.borrow().get_buffer().to_owned())
            .ok_or_else(|| RemoteError::custom(404, format!("No view for id {}", view_id), None))
    }

    fn do_set_language(&mut self, view_id: ViewId, language_id: LanguageId) {
        if let Some(view) = self.views.get(&view_id) {
            let buffer_id = view.borrow().get_buffer_id();
            let changes = self.config_manager.override_language(buffer_id, language_id.clone());

            let mut context = self.make_context(view_id).unwrap();
            context.language_changed(&language_id);
            if let Some(changes) = changes {
                context.config_changed(&changes);
            }
        }
    }

    fn do_start_plugin(&mut self, _view_id: ViewId, plugin: &str) {
        if self.running_plugins.iter().any(|p| p.name == plugin) {
            info!("plugin {} already running", plugin);
            return;
        }

        if let Some(manifest) = self.plugins.get_named(plugin) {
            //TODO: lots of races possible here, we need to keep track of
            //pending launches.
            start_plugin_process(
                manifest,
                self.next_plugin_id(),
                self.self_ref.as_ref().unwrap().clone(),
            );
        } else {
            warn!("no plugin found with name '{}'", plugin);
        }
    }

    fn do_stop_plugin(&mut self, _view_id: ViewId, plugin: &str) {
        if let Some(p) = self
            .running_plugins
            .iter()
            .position(|p| p.name == plugin)
            .map(|ix| self.running_plugins.remove(ix))
        {
            //TODO: verify shutdown; kill if necessary
            p.shutdown();
            self.after_stop_plugin(&p);
        }
    }

    fn do_plugin_rpc(&self, view_id: ViewId, receiver: &str, method: &str, params: &Value) {
        self.running_plugins
            .iter()
            .filter(|p| p.name == receiver)
            .for_each(|p| p.dispatch_command(view_id, method, params))
    }

    fn after_stop_plugin(&mut self, plugin: &Plugin) {
        self.iter_groups().for_each(|mut cx| cx.plugin_stopped(plugin));
    }
}

/// Idle, tracing, and file event handling
impl CoreState {
    pub(crate) fn handle_idle(&mut self, token: usize) {
        match token {
            NEW_VIEW_IDLE_TOKEN => self.finalize_new_views(),
            WATCH_IDLE_TOKEN => self.handle_fs_events(),
            other if (other & RENDER_VIEW_IDLE_MASK) != 0 => {
                self.handle_render_timer(other ^ RENDER_VIEW_IDLE_MASK)
            }
            other if (other & REWRAP_VIEW_IDLE_MASK) != 0 => {
                self.handle_rewrap_callback(other ^ REWRAP_VIEW_IDLE_MASK)
            }
            other if (other & FIND_VIEW_IDLE_MASK) != 0 => {
                self.handle_find_callback(other ^ FIND_VIEW_IDLE_MASK)
            }
            other => panic!("unexpected idle token {}", other),
        };
    }

    fn finalize_new_views(&mut self) {
        let to_start = mem::take(&mut self.pending_views);

        to_start.iter().for_each(|(id, config)| {
            let modified = self.detect_whitespace(*id, config);
            let config = modified.as_ref().unwrap_or(config);
            let mut edit_ctx = self.make_context(*id).unwrap();
            edit_ctx.finish_init(config);
        });
    }

    // Detects whitespace settings from the file and merges them with the config
    fn detect_whitespace(&mut self, id: ViewId, config: &Table) -> Option<Table> {
        let buffer_id = self.views.get(&id).map(|v| v.borrow().get_buffer_id())?;
        let editor = self
            .editors
            .get(&buffer_id)
            .expect("existing buffer_id must have corresponding editor");

        if editor.borrow().get_buffer().is_empty() {
            return None;
        }

        let autodetect_whitespace =
            self.config_manager.get_buffer_config(buffer_id).items.autodetect_whitespace;
        if !autodetect_whitespace {
            return None;
        }

        let mut changes = Table::new();
        let indentation = Indentation::parse(editor.borrow().get_buffer());
        match indentation {
            Ok(Some(Indentation::Tabs)) => {
                changes.insert("translate_tabs_to_spaces".into(), false.into());
            }
            Ok(Some(Indentation::Spaces(n))) => {
                changes.insert("translate_tabs_to_spaces".into(), true.into());
                changes.insert("tab_size".into(), n.into());
            }
            Err(_) => info!("detected mixed indentation"),
            Ok(None) => info!("file contains no indentation"),
        }

        let line_ending = LineEnding::parse(editor.borrow().get_buffer());
        match line_ending {
            Ok(Some(LineEnding::CrLf)) => {
                changes.insert("line_ending".into(), "\r\n".into());
            }
            Ok(Some(LineEnding::Lf)) => {
                changes.insert("line_ending".into(), "\n".into());
            }
            Err(_) => info!("detected mixed line endings"),
            Ok(None) => info!("file contains no supported line endings"),
        }

        let config_delta =
            self.config_manager.table_for_update(ConfigDomain::SysOverride(buffer_id), changes);
        match self
            .config_manager
            .set_user_config(ConfigDomain::SysOverride(buffer_id), config_delta)
        {
            Ok(ref mut items) if !items.is_empty() => {
                assert!(
                    items.len() == 1,
                    "whitespace overrides can only update a single buffer's config\n{:?}",
                    items
                );
                let table = items.remove(0).1;
                let mut config = config.clone();
                config.extend(table);
                Some(config)
            }
            Ok(_) => {
                warn!("set_user_config failed to update config, no tables were returned");
                None
            }
            Err(err) => {
                warn!("detect_whitespace failed to update config: {:?}", err);
                None
            }
        }
    }

    fn handle_render_timer(&mut self, token: usize) {
        let id: ViewId = token.into();
        if let Some(mut ctx) = self.make_context(id) {
            ctx._finish_delayed_render();
        }
    }

    /// Callback for doing word wrap on a view
    fn handle_rewrap_callback(&mut self, token: usize) {
        let id: ViewId = token.into();
        if let Some(mut ctx) = self.make_context(id) {
            ctx.do_rewrap_batch();
        }
    }

    /// Callback for doing incremental find in a view
    fn handle_find_callback(&mut self, token: usize) {
        let id: ViewId = token.into();
        if let Some(mut ctx) = self.make_context(id) {
            ctx.do_incremental_find();
        }
    }

    #[cfg(feature = "notify")]
    fn handle_fs_events(&mut self) {
        let _t = trace_block("CoreState::handle_fs_events", &["core"]);
        let mut events = self.file_manager.watcher().take_events();

        for (token, event) in events.drain(..) {
            match token {
                OPEN_FILE_EVENT_TOKEN => self.handle_open_file_fs_event(event),
                CONFIG_EVENT_TOKEN => self.handle_config_fs_event(event),
                THEME_FILE_EVENT_TOKEN => self.handle_themes_fs_event(event),
                PLUGIN_EVENT_TOKEN => self.handle_plugin_fs_event(event),
                _ => warn!("unexpected fs event token {:?}", token),
            }
        }
    }

    #[cfg(not(feature = "notify"))]
    fn handle_fs_events(&mut self) {}

    /// Handles a file system event related to a currently open file
    #[cfg(feature = "notify")]
    fn handle_open_file_fs_event(&mut self, event: Event) {
        use notify::event::*;
        let path = match event.kind {
            EventKind::Create(CreateKind::Any)
            | EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any))
            | EventKind::Modify(ModifyKind::Any) => &event.paths[0],
            other => {
                debug!("Ignoring event in open file {:?}", other);
                return;
            }
        };

        let buffer_id = match self.file_manager.get_editor(path) {
            Some(id) => id,
            None => return,
        };

        let has_changes = self.file_manager.check_file(path, buffer_id);
        let is_pristine = self.editors.get(&buffer_id).map(|ed| ed.borrow().is_pristine()).unwrap();
        //TODO: currently we only use the file's modification time when
        // determining if a file has been changed by another process.
        // A more robust solution would also hash the file's contents.

        if has_changes && is_pristine {
            if let Ok(text) = self.file_manager.open(path, buffer_id) {
                // this is ugly; we don't map buffer_id -> view_id anywhere
                // but we know we must have a view.
                let view_id = self
                    .views
                    .values()
                    .find(|v| v.borrow().get_buffer_id() == buffer_id)
                    .map(|v| v.borrow().get_view_id())
                    .unwrap();
                self.make_context(view_id).unwrap().reload(text);
            }
        }
    }

    /// Handles a config related file system event.
    #[cfg(feature = "notify")]
    fn handle_config_fs_event(&mut self, event: Event) {
        use notify::event::*;
        match event.kind {
            EventKind::Create(CreateKind::Any)
            | EventKind::Modify(ModifyKind::Any)
            | EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)) => {
                self.load_file_based_config(&event.paths[0])
            }
            EventKind::Remove(RemoveKind::Any) if !event.paths[0].exists() => {
                self.remove_config_at_path(&event.paths[0])
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
                self.remove_config_at_path(&event.paths[0]);
                self.load_file_based_config(&event.paths[1]);
            }
            _ => (),
        }
    }

    fn remove_config_at_path(&mut self, path: &Path) {
        if let Some(domain) = self.config_manager.domain_for_path(path) {
            self.set_config(domain, Table::default());
        }
    }

    /// Handles changes in plugin files.
    #[cfg(feature = "notify")]
    fn handle_plugin_fs_event(&mut self, event: Event) {
        use notify::event::*;
        match event.kind {
            EventKind::Create(CreateKind::Any) | EventKind::Modify(ModifyKind::Any) => {
                self.plugins.load_from_paths(&[event.paths[0].clone()]);
                if let Some(plugin) = self.plugins.get_from_path(&event.paths[0]) {
                    self.do_start_plugin(ViewId(0), &plugin.name);
                }
            }
            // the way FSEvents on macOS work, we want to verify that this path
            // has actually be removed before we do anything.
            EventKind::Remove(RemoveKind::Any) if !event.paths[0].exists() => {
                if let Some(plugin) = self.plugins.get_from_path(&event.paths[0]) {
                    self.do_stop_plugin(ViewId(0), &plugin.name);
                    self.plugins.remove_named(&plugin.name);
                }
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
                let old = &event.paths[0];
                let new = &event.paths[1];
                if let Some(old_plugin) = self.plugins.get_from_path(old) {
                    self.do_stop_plugin(ViewId(0), &old_plugin.name);
                    self.plugins.remove_named(&old_plugin.name);
                }

                self.plugins.load_from_paths(&[new.clone()]);
                if let Some(new_plugin) = self.plugins.get_from_path(new) {
                    self.do_start_plugin(ViewId(0), &new_plugin.name);
                }
            }
            EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any))
            | EventKind::Remove(RemoveKind::Any) => {
                if let Some(plugin) = self.plugins.get_from_path(&event.paths[0]) {
                    self.do_stop_plugin(ViewId(0), &plugin.name);
                    self.do_start_plugin(ViewId(0), &plugin.name);
                }
            }
            _ => (),
        }

        self.views.keys().for_each(|view_id| {
            let available_plugins = self
                .plugins
                .iter()
                .map(|plugin| ClientPluginInfo { name: plugin.name.clone(), running: true })
                .collect::<Vec<_>>();
            self.peer.available_plugins(*view_id, &available_plugins);
        });
    }

    /// Handles changes in theme files.
    #[cfg(feature = "notify")]
    fn handle_themes_fs_event(&mut self, event: Event) {
        use notify::event::*;
        match event.kind {
            EventKind::Create(CreateKind::Any) | EventKind::Modify(ModifyKind::Any) => {
                self.load_theme_file(&event.paths[0])
            }
            // the way FSEvents on macOS work, we want to verify that this path
            // has actually be removed before we do anything.
            EventKind::Remove(RemoveKind::Any) if !event.paths[0].exists() => {
                self.remove_theme(&event.paths[0]);
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
                let old = &event.paths[0];
                let new = &event.paths[1];
                self.remove_theme(old);
                self.load_theme_file(new);
            }
            EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any))
            | EventKind::Remove(RemoveKind::Any) => {
                self.style_map.borrow_mut().sync_dir(event.paths[0].parent())
            }
            _ => (),
        }
        let theme_names = self.style_map.borrow().get_theme_names();
        self.peer.available_themes(theme_names);
    }

    /// Load a single theme file. Updates if already present.
    fn load_theme_file(&mut self, path: &Path) {
        let _t = trace_block("CoreState::load_theme_file", &["core"]);

        let result = self.style_map.borrow_mut().load_theme_info_from_path(path);
        match result {
            Ok(theme_name) => {
                if theme_name == self.style_map.borrow().get_theme_name() {
                    if self.style_map.borrow_mut().set_theme(&theme_name).is_ok() {
                        self.notify_client_and_update_views();
                    }
                }
            }
            Err(e) => error!("Error loading theme file: {:?}, {:?}", path, e),
        }
    }

    fn remove_theme(&mut self, path: &Path) {
        let result = self.style_map.borrow_mut().remove_theme(path);

        // Set default theme if the removed theme was the
        // current one.
        if let Some(theme_name) = result {
            if theme_name == self.style_map.borrow().get_theme_name() {
                self.do_set_theme(DEFAULT_THEME);
            }
        }
    }

    fn toggle_tracing(&self, enabled: bool) {
        self.running_plugins.iter().for_each(|plugin| plugin.toggle_tracing(enabled))
    }

    fn save_trace<P>(&self, path: P, frontend_samples: Value)
    where
        P: AsRef<Path>,
    {
        use xi_trace::chrome_trace_dump;
        let mut all_traces = xi_trace::samples_cloned_unsorted();
        if let Ok(mut traces) = chrome_trace_dump::decode(frontend_samples) {
            all_traces.append(&mut traces);
        }

        for plugin in &self.running_plugins {
            match plugin.collect_trace() {
                Ok(json) => {
                    let mut trace = chrome_trace_dump::decode(json).unwrap();
                    all_traces.append(&mut trace);
                }
                Err(e) => error!("trace error {:?}", e),
            }
        }

        all_traces.sort_unstable();

        let mut trace_file = match File::create(path.as_ref()) {
            Ok(f) => f,
            Err(e) => {
                error!("error saving trace {:?}", e);
                return;
            }
        };

        if let Err(e) = chrome_trace_dump::serialize(&all_traces, &mut trace_file) {
            error!("error saving trace {:?}", e);
        }
    }
}

/// plugin event handling
impl CoreState {
    /// Called from a plugin's thread after trying to start the plugin.
    pub(crate) fn plugin_connect(&mut self, plugin: Result<Plugin, io::Error>) {
        match plugin {
            Ok(plugin) => {
                let init_info =
                    self.iter_groups().map(|mut ctx| ctx.plugin_info()).collect::<Vec<_>>();
                plugin.initialize(init_info);
                self.running_plugins.push(plugin);
            }
            Err(e) => error!("failed to start plugin {:?}", e),
        }
    }

    pub(crate) fn plugin_exit(&mut self, id: PluginId, error: Result<(), ReadError>) {
        warn!("plugin {:?} exited with result {:?}", id, error);
        let running_idx = self.running_plugins.iter().position(|p| p.id == id);
        if let Some(idx) = running_idx {
            let plugin = self.running_plugins.remove(idx);
            self.after_stop_plugin(&plugin);
        }
    }

    /// Handles the response to a sync update sent to a plugin.
    pub(crate) fn plugin_update(
        &mut self,
        _plugin_id: PluginId,
        view_id: ViewId,
        response: Result<Value, xi_rpc::Error>,
    ) {
        if let Some(mut edit_ctx) = self.make_context(view_id) {
            edit_ctx.do_plugin_update(response);
        }
    }

    pub(crate) fn plugin_notification(
        &mut self,
        _ctx: &RpcCtx,
        view_id: ViewId,
        plugin_id: PluginId,
        cmd: PluginNotification,
    ) {
        if let Some(mut edit_ctx) = self.make_context(view_id) {
            edit_ctx.do_plugin_cmd(plugin_id, cmd)
        }
    }

    pub(crate) fn plugin_request(
        &mut self,
        _ctx: &RpcCtx,
        view_id: ViewId,
        plugin_id: PluginId,
        cmd: PluginRequest,
    ) -> Result<Value, RemoteError> {
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
    use super::{BufferId, ViewId};

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

impl<'a, I> Iterator for Iter<'a, I>
where
    I: Iterator<Item = &'a ViewId>,
{
    type Item = EventContext<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let &mut Iter { ref mut views, ref mut seen, inner } = self;
        loop {
            let next_view = match views.next() {
                None => return None,
                Some(v) if seen.contains(v) => continue,
                Some(v) => v,
            };
            let context = inner.make_context(*next_view).unwrap();
            context.siblings.iter().for_each(|sibl| {
                let _ = seen.insert(sibl.borrow().get_view_id());
            });
            return Some(context);
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Counter(Cell<usize>);

impl Counter {
    pub(crate) fn next(&self) -> usize {
        let n = self.0.get();
        self.0.set(n + 1);
        n + 1
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
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ViewId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.trim_start_matches("view-id-")
            .parse::<usize>()
            .map(ViewId)
            .map_err(|_| de::Error::invalid_value(Unexpected::Str(&s), &"view id"))
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

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::ViewId;

    #[test]
    fn test_deserialize_view_id() {
        let de = json!("view-id-1");
        assert_eq!(ViewId::deserialize(&de).unwrap(), ViewId(1));

        let de = json!("not-a-view-id");
        assert!(ViewId::deserialize(&de).unwrap_err().is_data());
    }
}
