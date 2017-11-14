// Copyright 2017 Google Inc. All rights reserved.
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

//! `PluginManager` handles launching, monitoring, and communicating with plugins.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak, MutexGuard};

use std::path::Path;
use std::fmt::Debug;

use serde::Serialize;
use serde_json::{self, Value};

use tabs::{BufferIdentifier, ViewIdentifier, BufferContainerRef};

use super::{PluginCatalog, PluginRef, start_plugin_process, PluginPid};
use super::rpc_types::{PluginNotification, PluginRequest, PluginUpdate, UpdateResponse, PluginBufferInfo, ClientPluginInfo};
use super::manifest::{PluginActivation, Command};

pub type PluginName = String;
type PluginGroup = BTreeMap<PluginName, PluginRef>;

/// Manages plugin loading, activation, lifecycle, and dispatch.
pub struct PluginManager {
    catalog: PluginCatalog,
    /// Buffer-scoped plugins, by buffer
    buffer_plugins: BTreeMap<BufferIdentifier, PluginGroup>,
    global_plugins: PluginGroup,
    buffers: BufferContainerRef,
    next_id: usize,
}

#[derive(Debug)]
/// The error type for plugin operations.
pub enum Error {
    /// There was an error finding the buffer associated with a given view.
    /// This probably means the buffer was destroyed while an RPC was in flight.
    EditorMissing,
    /// An error launching or communicating with a plugin process.
    IoError(io::Error),
    Other(String),
}

// Note: In general I have adopted the pattern of putting non-threadsafe
// API in the 'inner' type of types with a ___Ref variant. Methods on the
// Ref variant should be minimal, and provide a threadsafe interface.

impl PluginManager {

    /// Returns plugins available to this view.
    pub fn get_available_plugins(&self, view_id: ViewIdentifier) -> Vec<ClientPluginInfo> {
        self.catalog.iter_names().map(|name| {
            let running = self.plugin_is_running(view_id, &name);
            let name = name.clone();
            ClientPluginInfo { name, running }
        }).collect::<Vec<_>>()
    }

    /// Handle a request from a plugin.
    pub fn handle_plugin_notification(&self, cmd: PluginNotification, plugin_id: PluginPid) {
        use self::PluginNotification::*;
        match cmd {
            //TODO: these should not be unwraps
            AddScopes { view_id, scopes } => {
                self.buffers.lock().editor_for_view_mut(view_id).unwrap()
                    .plugin_add_scopes(plugin_id, scopes);
            }
            UpdateSpans { view_id, start, len, spans, rev } => {
                self.buffers.lock().editor_for_view_mut(view_id).unwrap()
                    .plugin_update_spans(plugin_id, start, len, spans, rev);
            }
            Edit { view_id, edit } => {
                self.buffers.lock().editor_for_view_mut(view_id).unwrap()
                    .plugin_edit(&edit);
            }
            Alert { view_id, msg } => {
                self.buffers.lock().editor_for_view(view_id).unwrap()
                .plugin_alert(&msg);
            }
        }
    }

    /// Handle a request from a plugin.
    pub fn handle_plugin_request(&self, cmd: PluginRequest, _: PluginPid) -> Value {
        use self::PluginRequest::*;
        match cmd {
            //TODO: these should not be unwraps
            LineCount { view_id } => {
                let n_lines = self.buffers.lock().editor_for_view(view_id).unwrap()
                    .plugin_n_lines() as u64;
                serde_json::to_value(n_lines).unwrap()
            }
            GetData { view_id, offset, max_size, rev } => {
                self.buffers.lock().editor_for_view(view_id).unwrap()
                .plugin_get_data(offset, max_size, rev)
                .map(|data| Value::String(data)).unwrap()
            }
            GetSelections { view_id } => {
                self.buffers.lock().editor_for_view(view_id).unwrap()
                    .plugin_get_selections(view_id)
            }
        }
    }

    /// Passes an update from a buffer to all registered plugins.
    fn update_plugins(&mut self, view_id: ViewIdentifier,
                  update: PluginUpdate, undo_group: usize) -> Result<(), Error> {

        // find all running plugins for this buffer, and send them the update
        let mut dead_plugins = Vec::new();

        if let Ok(running) = self.running_for_view(view_id) {
            for (name, plugin) in running.iter().chain(self.global_plugins.iter()) {
                // check to see if plugins have crashed
                if plugin.is_dead() {
                    dead_plugins.push((name.to_owned(), plugin.get_identifier()));
                    continue;
                }

                self.buffers.lock().editor_for_view_mut(view_id)
                    .unwrap().increment_revs_in_flight();

                let view_id = view_id.to_owned();
                let buffers = self.buffers.clone().to_weak();
                let mut plugin_ref = plugin.clone();

                plugin.update(&update, move |response| {
                    let buffers = match buffers.upgrade() {
                        Some(b) => b,
                        None => return,
                    };

                    match response.map(serde_json::from_value::<UpdateResponse>) {
                        Ok(Ok(UpdateResponse::Edit(edit))) => {
                            buffers.lock().editor_for_view_mut(view_id).unwrap()
                                .apply_plugin_edit(&edit, Some(undo_group));
                        }
                        Ok(Ok(UpdateResponse::Ack(_))) => (),
                        Ok(Err(err)) => eprintln!("plugin response json err: {:?}", err),
                        Err(err) => {
                            eprintln!("plugin process dead? {:?}", err);
                            //TODO: do we have a retry policy?
                            plugin_ref.declare_dead();
                        }
                    }
                    buffers.lock().editor_for_view_mut(view_id)
                        .unwrap().dec_revs_in_flight();
                });
            }
        };
        self.cleanup_dead(view_id, &dead_plugins);
        self.buffers.lock().editor_for_view_mut(view_id).unwrap().dec_revs_in_flight();
        Ok(())
    }

    /// Sends a notification to groups of plugins.
    fn notify_plugins<V>(&self, view_id: ViewIdentifier,
                         only_globals: bool, method: &str, params: &V)
        where V: Serialize + Debug
    {
        let params = serde_json::to_value(params)
            .expect(&format!("bad notif params.\nmethod: {}\nparams: {:?}",
                             method, params));
        for (_, plugin) in self.global_plugins.iter() {
            plugin.rpc_notification(method, &params);
        }
        if !only_globals {
            if let Ok(locals) = self.running_for_view(view_id) {
                for (_, plugin) in locals {
                    plugin.rpc_notification(method, &params);
                }
            }
        }
    }

    fn dispatch_command(&self, view_id: ViewIdentifier, receiver: &str,
                        method: &str, params: &Value) {
        let plugin_ref = self.running_for_view(view_id)
            .ok()
            .and_then(|r| r.get(receiver));

        match plugin_ref {
            Some(plug) => {
                let inner = json!({"method": method, "params": params});
                plug.rpc_notification("custom_command", &inner);
            }
            None => {
                eprintln!("missing plugin {} for command {}", receiver, method);
            }
        }
    }

    /// Launches and initializes the named plugin.
    fn start_plugin(&mut self,
                    self_ref: &PluginManagerRef,
                    view_id: ViewIdentifier,
                    init_info: &PluginBufferInfo,
                    plugin_name: &str, ) -> Result<(), Error> {

        // verify that this view_id is valid
         let _ = self.running_for_view(view_id)?;
         if self.plugin_is_running(view_id, plugin_name) {
             return Err(Error::Other(format!("{} already running", plugin_name)));
         }

        let plugin_id = self.next_plugin_id();
        let plugin_desc = self.catalog.get_named(plugin_name)
            .ok_or(Error::Other(format!("no plugin found with name {}", plugin_name)))?;

        let is_global = plugin_desc.is_global();
        let commands = plugin_desc.commands.clone();
        let init_info = if is_global {
            let buffers = self.buffers.lock();
            let info = buffers.iter_editors()
                .map(|ed| ed.plugin_init_info().to_owned())
                .collect::<Vec<_>>();
            info
        } else {
            vec![init_info.to_owned()]
        };

        let me = self_ref.clone();
        let plugin_name = plugin_name.to_owned();

        start_plugin_process(self_ref, &plugin_desc, plugin_id, move |result| {
            match result {
                Ok(plugin_ref) => {
                    plugin_ref.initialize(&init_info);
                    if is_global {
                        me.lock().on_plugin_connect_global(&plugin_name, plugin_ref,
                                                           commands);
                    } else {
                        me.lock().on_plugin_connect_local(view_id, &plugin_name,
                                                          plugin_ref, commands);
                    }
                }
                Err(err) => eprintln!("failed to start plugin {}:\n {:?}",
                                     plugin_name, err),
            }
        });
        Ok(())
    }

    /// Callback used to register a successfully launched local plugin.
    fn on_plugin_connect_local(&mut self, view_id: ViewIdentifier,
                              plugin_name: &str, plugin_ref: PluginRef,
                              commands: Vec<Command>) {
        // only add to our 'running' collection if the editor still exists
        let is_running = match self.buffers.lock().editor_for_view(view_id) {
            Some(ed) => {
                ed.plugin_started(view_id, plugin_name, &commands);
                true
            }
            None => false,
        };
        if is_running {
            let _ = self.running_for_view_mut(view_id)
                .map(|running| running.insert(plugin_name.to_owned(), plugin_ref));
        } else {
            eprintln!("launch of plugin {} failed, no buffer for view {}",
                       plugin_name, view_id);
            plugin_ref.shutdown();
        }
    }

    /// Callback used to register a successfully launched global plugin.
    fn on_plugin_connect_global(&mut self, plugin_name: &str,
                                plugin_ref: PluginRef, commands: Vec<Command>) {
        {
            let buffers = self.buffers.lock();
            for ed in buffers.iter_editors() {
                ed.plugin_started(None, plugin_name, &commands);
            }
        }
        self.global_plugins.insert(plugin_name.to_owned(), plugin_ref);
    }

    fn stop_plugin(&mut self, view_id: ViewIdentifier, plugin_name: &str) {
        let is_global = self.catalog.get_named(plugin_name).unwrap().is_global();
        if is_global {
            let plugin_ref = self.global_plugins.remove(plugin_name);
            if let Some(plugin_ref) = plugin_ref {
                let plugin_id = plugin_ref.get_identifier();
                plugin_ref.shutdown();
                let mut buffers = self.buffers.lock();
                for ed in buffers.iter_editors_mut() {
                    ed.plugin_stopped(None, plugin_name, plugin_id, 0);
                }
            }
        }
        let plugin_ref = match self.running_for_view_mut(view_id) {
            Ok(running) => running.remove(plugin_name),
            Err(_) => None,
        };

        if let Some(plugin_ref) = plugin_ref {
            let plugin_id = plugin_ref.get_identifier();
            plugin_ref.shutdown();
            //TODO: should we notify now, or wait until we know this worked?
            //can this fail? (yes.) How do we tell, and when do we kill the proc?
            if let Some(ed) = self.buffers.lock().editor_for_view_mut(view_id) {
                ed.plugin_stopped(view_id, plugin_name, plugin_id, 0);
            }
        }
    }

    /// Remove dead plugins, notifying editors as needed.
    //TODO: this currently only runs after trying to update a plugin that has crashed
    // during a previous update: that is, if a plugin crashes it isn't cleaned up
    // immediately. If this is a problem, we should store crashes, and clean up in idle().
    #[allow(non_snake_case)]
    fn cleanup_dead(&mut self, view_id: ViewIdentifier, plugins: &[(PluginName, PluginPid)]) {
        //TODO: define exit codes, put them in an enum somewhere
        let ABNORMAL_EXIT_CODE = 1;
        for &(ref name, pid) in plugins.iter() {
            let is_global = self.catalog.get_named(name).unwrap().is_global();
            if is_global {
                {
                    self.global_plugins.remove(name);
                }
                let mut buffers = self.buffers.lock();
                for ed in buffers.iter_editors_mut() {
                    ed.plugin_stopped(None, name, pid, ABNORMAL_EXIT_CODE);
                }

            } else {
                let _ = self.running_for_view_mut(view_id)
                    .map(|running| running.remove(name));
                self.buffers.lock().editor_for_view_mut(view_id).map(|ed|{
                    ed.plugin_stopped(view_id, name, pid, ABNORMAL_EXIT_CODE);
                });
            }
        }
    }

    // ====================================================================
    // convenience functions
    // ====================================================================

    fn buffer_for_view(&self, view_id: ViewIdentifier) -> Option<BufferIdentifier> {
        self.buffers.buffer_for_view(view_id).map(|id| id.to_owned())
    }

    fn next_plugin_id(&mut self) -> PluginPid {
        self.next_id += 1;
        PluginPid(self.next_id)
    }

    fn plugin_is_running(&self, view_id: ViewIdentifier, plugin_name: &str) -> bool {
        self.buffer_for_view(view_id)
            .and_then(|id| self.buffer_plugins.get(&id))
            .map(|plugins| plugins.contains_key(plugin_name))
            .unwrap_or_default() || self.global_plugins.contains_key(plugin_name)
    }

    // TODO: we have a bunch of boilerplate around handling the rare case
    // where we receive a command for some buffer which no longer exists.
    // Maybe these two functions should return a Box<Iterator> of plugins,
    // and if the buffer is missing just print a debug message and return
    // an empty Iterator?
    fn running_for_view(&self, view_id: ViewIdentifier) -> Result<&PluginGroup, Error> {
        self.buffer_for_view(view_id)
            .and_then(|id| self.buffer_plugins.get(&id))
            .ok_or(Error::EditorMissing)
    }

    fn running_for_view_mut(&mut self, view_id: ViewIdentifier) -> Result<&mut PluginGroup, Error> {
        let buffer_id = match self.buffer_for_view(view_id) {
            Some(id) => Ok(id),
            None => Err(Error::EditorMissing),
        }?;
        self.buffer_plugins.get_mut(&buffer_id)
            .ok_or(Error::EditorMissing)
    }
}

/// Wrapper around an `Arc<Mutex<PluginManager>>`.
pub struct PluginManagerRef(Arc<Mutex<PluginManager>>);

impl Clone for PluginManagerRef {
    fn clone(&self) -> Self {
        PluginManagerRef(self.0.clone())
    }
}

/// Wrapper around a `Weak<Mutex<PluginManager>>`
pub struct WeakPluginManagerRef(Weak<Mutex<PluginManager>>);

impl WeakPluginManagerRef {
    /// Upgrades the weak reference to an Arc, if possible.
    ///
    /// Returns `None` if the inner value has been deallocated.
    pub fn upgrade(&self) -> Option<PluginManagerRef> {
        match self.0.upgrade() {
            Some(inner) => Some(PluginManagerRef(inner)),
            None => None
        }
    }
}

impl PluginManagerRef {
    pub fn new(buffers: BufferContainerRef) -> Self {
        PluginManagerRef(Arc::new(Mutex::new(
            PluginManager {
                // TODO: actually parse these from manifest files
                catalog: PluginCatalog::from_paths(Vec::new()),
                buffer_plugins: BTreeMap::new(),
                global_plugins: PluginGroup::new(),
                buffers: buffers,
                next_id: 0,
            }
        )))
    }

    pub fn lock(&self) -> MutexGuard<PluginManager> {
        self.0.lock().unwrap()
    }

    /// Creates a new `WeakPluginManagerRef`.
    pub fn to_weak(&self) -> WeakPluginManagerRef {
        WeakPluginManagerRef(Arc::downgrade(&self.0))
    }

    pub fn set_plugin_search_path(&self, paths: Vec<PathBuf>) {
        // hacky: we don't handle unloading plugins if the path changes.
        // this assumes that we only set the path once, when we get
        // `client_init`.
        let mut inner = self.lock();
        assert!(inner.catalog.iter().count() == 0);
        inner.catalog = PluginCatalog::from_paths(paths);

    }


    /// Called when a new buffer is created.
    pub fn document_new(&self, view_id: ViewIdentifier, init_info: &PluginBufferInfo) {
        let available = self.lock().get_available_plugins(view_id);
        {
            let inner = self.lock();
            let buffers = inner.buffers.lock();
            buffers.editor_for_view(view_id)
                .map(|ed| { ed.available_plugins(view_id, &available) });
        }

        if self.add_running_collection(view_id).is_ok() {
            let to_start = self.activatable_plugins(view_id);
            self.start_plugins(view_id, &init_info, &to_start);
            self.lock().notify_plugins(view_id, true, "new_buffer", &json!({
                "buffer_info": vec![&init_info],
            }));
        }
    }

    /// Called when a buffer is saved to a file.
    pub fn document_did_save(&self, view_id: ViewIdentifier, path: &Path) {
        self.lock().notify_plugins(view_id, false, "did_save", &json!({
            "view_id": view_id,
            "path": path,
        }));
    }

    /// Called when a buffer is closed.
    pub fn document_close(&self, view_id: ViewIdentifier) {
        let to_stop = self.lock().running_for_view(view_id)
            .map(|running| {
                running.keys()
                    .map(|k| k.to_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        for plugin_name in to_stop {
            self.stop_plugin(view_id, &plugin_name);
        }
        self.lock().notify_plugins(view_id, true, "did_close", &json!({
            "view_id": view_id}));
    }

    /// Called when a document's syntax definition has changed.
    pub fn document_syntax_changed(&self, view_id: ViewIdentifier, init_info: PluginBufferInfo) {
        eprintln!("document_syntax_changed {}", view_id);

        let start_keys = self.activatable_plugins(view_id).iter()
            .map(|p| p.to_owned())
            .collect::<BTreeSet<_>>();

        // stop currently running plugins that aren't on list
        // TODO: don't stop plugins that weren't started by a syntax activation
        for plugin_name in self.lock().running_for_view(view_id)
            .unwrap()
            .keys()
            .filter(|k| !start_keys.contains(*k)) {
                self.stop_plugin(view_id, &plugin_name);
            }

        //TODO: send syntax_changed notification before starting new plugins

        let to_run = start_keys.iter()
            .filter(|k| !self.lock().plugin_is_running(view_id, k))
            .map(|k| k.clone().to_owned())
            .collect::<Vec<String>>();

        self.start_plugins(view_id, &init_info, &to_run);
    }

    /// Launches and initializes the named plugin.
    pub fn start_plugin(&self,
                        view_id: ViewIdentifier,
                        init_info: &PluginBufferInfo,
                        plugin_name: &str) -> Result<(), Error> {
        self.lock().start_plugin(self, view_id, init_info, plugin_name)
    }

    /// Terminates and cleans up the named plugin.
    pub fn stop_plugin(&self, view_id: ViewIdentifier, plugin_name: &str) {
        self.lock().stop_plugin(view_id, plugin_name);
    }

    /// Forward an update from a view to registered plugins.
    pub fn update_plugins(&self, view_id: ViewIdentifier,
                          update: PluginUpdate, undo_group: usize) -> Result<(), Error> {
        self.lock().update_plugins(view_id, update, undo_group)
    }

    /// Sends a custom notification to a running plugin
    pub fn dispatch_command(&self, view_id: ViewIdentifier, receiver: &str,
                             method: &str, params: &Value) {
        self.lock().dispatch_command(view_id, receiver, method, params);
    }

    // ====================================================================
    // implementation details
    // ====================================================================

    /// Performs new buffer setup.
    ///
    /// Returns an error if `view_id` does not have an associated buffer,
    /// which is possible if it was closed immediately after creation.
    fn add_running_collection(&self, view_id: ViewIdentifier) -> Result<(),()> {
        assert!(self.lock().running_for_view(view_id).is_err());
        let buf_id = self.lock().buffer_for_view(view_id);
        match buf_id {
            Some(buf_id) => {
                self.lock().buffer_plugins.insert(buf_id, PluginGroup::new());
                Ok(())
            }
            None => Err(())
        }
    }

    /// Returns the plugins which want to activate for this view.
    ///
    /// That a plugin wants to activate does not mean it will be activated.
    /// For instance, it could have already been disabled by user preference.
    fn activatable_plugins(&self, view_id: ViewIdentifier) -> Vec<String> {
        let inner = self.lock();
        let syntax = inner.buffers.lock()
            .editor_for_view(view_id)
            .unwrap()
            .get_syntax()
            .to_owned();

        inner.catalog.filter(|plug_desc|{
            plug_desc.activations.iter().any(|act|{
                match *act {
                    PluginActivation::Autorun => true,
                    PluginActivation::OnSyntax(ref other) if *other == syntax => true,
                    _ => false,
                }
            })
        })
        .iter()
        .map(|desc| desc.name.to_owned())
        .collect::<Vec<_>>()
    }

    /// Batch run a group of plugins (as on creating a new view, for instance)
    fn start_plugins(&self, view_id: ViewIdentifier,
                     init_info: &PluginBufferInfo, plugin_names: &Vec<String>) {
        eprintln!("starting plugins for {}", view_id);
        for plugin_name in plugin_names.iter() {
            match self.start_plugin(view_id, init_info, plugin_name) {
                Ok(_) => eprintln!("starting plugin {}", plugin_name),
                Err(err) => eprintln!("unable to start plugin {}, err: {:?}",
                                       plugin_name, err),
            }
        }
    }
}
