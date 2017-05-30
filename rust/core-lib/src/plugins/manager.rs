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
use std::io::Write;
use std::sync::{Arc, Mutex, Weak, MutexGuard};

use serde_json::{self, Value};

use tabs::{BufferIdentifier, ViewIdentifier, BufferContainerRef};

use super::{PluginDescription, PluginRef, start_plugin};
use super::rpc_types::{PluginCommand, PluginUpdate, UpdateResponse, ClientPluginInfo, PluginBufferInfo};
use super::manifest::{PluginActivation, debug_plugins};

type PluginName = String;

/// Manages plugin loading, activation, lifecycle, and dispatch.
pub struct PluginManager<W: Write> {
    catalog: Vec<PluginDescription>,
    running: BTreeMap<(BufferIdentifier, PluginName), PluginRef<W>>,
    buffers: BufferContainerRef<W>,
}

// Note: In general I have adopted the pattern of putting non-threadsafe
// API in the 'inner' type of types with a ___Ref variant. Methods on the
// Ref variant should be minimal, and provide a threadsafe interface.

impl <W: Write + Send + 'static>PluginManager<W> {

    /// Returns plugins available to this view.
    pub fn available_plugins(&self, view_id: &ViewIdentifier) -> Vec<ClientPluginInfo> {
        self.catalog.iter().map(|p| {
            let buffer_id = self.buffer_for_view(view_id);
            let key = (buffer_id, p.name.clone());
            let running = self.running.contains_key(&key);
            let name = key.1;
            ClientPluginInfo { name, running }
        }).collect::<Vec<_>>()
    }

    /// Handle a request from a plugin.
    pub fn handle_plugin_cmd(&self, cmd: PluginCommand, view_id: &ViewIdentifier,
                             plugin_id: &str) -> Option<Value> {
        use self::PluginCommand::*;
        match cmd {
            LineCount => {
                let n_lines = self.buffers.lock().editor_for_view(view_id).unwrap()
                    .plugin_n_lines() as u64;
                Some(serde_json::to_value(n_lines).unwrap())
            },
            SetFgSpans { .. } => {
                print_err!("set_fg_spans has been removed; use update_spans.");
                None
            }
            AddScopes { scopes } => {
                self.buffers.lock().editor_for_view_mut(view_id).unwrap()
                    .plugin_add_scopes(plugin_id, scopes);
                None
            }
            UpdateSpans { start, len, spans, rev } => {
                self.buffers.lock().editor_for_view_mut(view_id).unwrap()
                    .plugin_update_spans(plugin_id, start, len, spans, rev);
                None
            }
            GetData { offset, max_size, rev } => {
                self.buffers.lock().editor_for_view(view_id).unwrap()
                .plugin_get_data(offset, max_size, rev)
                .map(|data| Value::String(data))
            }
            Alert { msg } => {
                self.buffers.lock().editor_for_view(view_id).unwrap()
                .plugin_alert(&msg);
                None
            }
        }
    }

    /// Passes an update from a buffer to all registered plugins.
    pub fn update(&mut self, view_id: &ViewIdentifier,
                  update: PluginUpdate, undo_group: usize) {
        // find all running plugins for this buffer, and send them the update
        let buffer_id = self.buffer_for_view(view_id);
        let mut dead_plugins = Vec::new();

        for (key, plugin) in self.running.iter().filter(|kv| &(kv.0).0 == &buffer_id) {
            // check to see if plugins have crashed
            if plugin.is_dead() {
                dead_plugins.push(key.to_owned());
                continue
            }
            self.buffers.lock().editor_for_view_mut(view_id)
                .unwrap().increment_revs_in_flight();

            let view_id = view_id.to_owned();
            let buffers = self.buffers.clone().to_weak();
            let mut plugin_ref = plugin.clone();
            plugin.update(&update, move |response| {
                if let Some(buffers) = buffers.upgrade() {
                    match response.map(|v| serde_json::from_value::<UpdateResponse>(v)) {
                        Ok(Ok(UpdateResponse::Edit(edit))) => {
                            buffers.lock().editor_for_view_mut(&view_id).unwrap()
                                .apply_plugin_edit(edit, undo_group);
                        }
                        Ok(Ok(UpdateResponse::Ack(_))) => (),
                        Ok(Err(err)) => print_err!("plugin response json err: {:?}", err),
                        Err(err) => {
                            print_err!("plugin process dead? {:?}", err);
                            //TODO: do we have a retry policy?
                            plugin_ref.declare_dead();
                        }
                    }
                    buffers.lock().editor_for_view_mut(&view_id)
                        .unwrap().dec_revs_in_flight();
                }
            });
        }

        for key in dead_plugins {
            self.running.remove(&key);
            self.buffers.lock().editor_for_view(&view_id).map(|ed|{
                ed.plugin_stopped(&view_id, &key.1, 1);
            });
        }
        self.buffers.lock().editor_for_view_mut(view_id).unwrap().dec_revs_in_flight();
    }

    /// Launches and initializes the named plugin.
    pub fn start_plugin(&mut self, self_ref: &PluginManagerRef<W>,
                          view_id: &ViewIdentifier, plugin_name: &str, init_info: PluginBufferInfo) {
        //TODO: error handling: this should maybe have a completion callback with a Result
        let buffer_id = self.buffers.buffer_for_view(view_id);
        if let Some(buffer_id) = buffer_id {
            let key = (buffer_id.to_owned(), plugin_name.to_owned());
            if self.running.contains_key(&key) {
                return print_err!("plugin {} already running for buffer {}", plugin_name, key.0);
            }

            let plugin = self.catalog.iter().find(|desc| desc.name == plugin_name)
                .expect(&format!("no plugin found with name {}", plugin_name));

            let me = self_ref.clone();
            let view_id2 = view_id.to_owned();

            start_plugin(self_ref, &plugin, &view_id, move |result| {
                match result {
                    Ok(plugin_ref) => {
                        plugin_ref.initialize(&init_info);
                        let mut inner = me.lock();
                        let mut running = false;

                        if let Some(ed) = inner.buffers.lock()
                            .editor_for_view(&view_id2) {
                                ed.plugin_started(&view_id2, &key.1);
                                running = true;
                        }
                        if running {
                            inner.running.insert(key, plugin_ref);
                        }
                    }
                    Err(_) => panic!("error handling is not implemented"),
                }
            });
        }
        else {
            print_err!("couldn't start plugin {}, no buffer found for view {}",
                       plugin_name, view_id)
        }
    }

    fn stop_plugin(&mut self, view_id: &ViewIdentifier, plugin_name: &str) {
        if let Some(buffer_id) = self.buffers.buffer_for_view(view_id) {
            let key = (buffer_id.to_owned(), plugin_name.to_owned());
            match self.running.remove(&key) {
                Some(plugin) => {
                    plugin.shutdown();
                    //TODO: should we notify now, or wait until we know this worked?
                    //can this fail? (yes.) How do we tell, and when do we kill the proc?
                    if let Some(editor) = self.buffers.lock().editor_for_view(view_id) {
                        editor.plugin_stopped(view_id, plugin_name, 0);
                    }
                }
                None => print_err!("stop_plugin: plugin not running for buffer {} {}",
                                   plugin_name, view_id),
            }
        }
    }

    fn buffer_for_view(&self, view_id: &ViewIdentifier) -> BufferIdentifier {
        let buffer_id = self.buffers.buffer_for_view(view_id);
        if buffer_id.is_none() { print_err!("no buffer for view {}", view_id) }
        buffer_id.unwrap_or(BufferIdentifier::from("null-id"))
    }
}

/// Wrapper around a `Arc<Mutex<PluginManager<W>>>`.
pub struct PluginManagerRef<W: Write>(Arc<Mutex<PluginManager<W>>>);

impl<W: Write> Clone for PluginManagerRef<W> {
    fn clone(&self) -> Self {
        PluginManagerRef(self.0.clone())
    }
}

/// Wrapper around a `Weak<Mutex<PluginManager<W>>>`
pub struct WeakPluginManagerRef<W: Write>(Weak<Mutex<PluginManager<W>>>);

impl <W: Write>WeakPluginManagerRef<W> {
    /// Upgrades the weak reference to an Arc, if possible.
    ///
    /// Returns `None` if the inner value has been deallocated.
    pub fn upgrade(&self) -> Option<PluginManagerRef<W>> {
        match self.0.upgrade() {
            Some(inner) => Some(PluginManagerRef(inner)),
            None => None
        }
    }
}

impl<W: Write + Send + 'static> PluginManagerRef<W> {
    pub fn new(buffers: BufferContainerRef<W>) -> Self {
        PluginManagerRef(Arc::new(Mutex::new(
        PluginManager {
            // TODO: actually parse these from manifest files
            catalog: debug_plugins(),
            running: BTreeMap::new(),
            buffers: buffers,
        })))
    }

    pub fn lock(&self) -> MutexGuard<PluginManager<W>> {
        self.0.lock().unwrap()
    }

    /// Creates a new `WeakPluginManagerRef<W>`.
    pub fn to_weak(&self) -> WeakPluginManagerRef<W> {
        let weak_inner = Arc::downgrade(&self.0);
        WeakPluginManagerRef(weak_inner)
    }


    /// Called when a new empty buffer is created.
    pub fn document_new(&mut self, view_id: &ViewIdentifier) {
        print_err!("document new {}", view_id);
        let to_start = self.activatable_plugins(view_id);
        self.start_plugins(view_id, &to_start);
        //TODO: send lifecycle notification
    }

    /// Called when an existing file is loaded into a buffer.
    pub fn document_open(&mut self, view_id: &ViewIdentifier) {
        print_err!("document open {}", view_id);
        let to_start = self.activatable_plugins(view_id);
        self.start_plugins(view_id, &to_start);
        //TODO: send lifecycle notification
    }

    /// Called when a buffer is saved to a file.
    pub fn document_did_save(&mut self, view_id: &ViewIdentifier) {
        print_err!("document_did_save {}", view_id);
    }

    /// Called when a buffer is closed.
    pub fn document_close(&mut self, view_id: &ViewIdentifier) {
        let buffer_id = self.lock().buffer_for_view(view_id);
        let to_stop = self.lock().running.keys()
            .filter(|k| k.0 == buffer_id)
            .map(|k| k.1.to_owned())
            .collect::<Vec<_>>();
        for plugin_name in to_stop {
            self.stop_plugin(view_id, &plugin_name);
        }
        //TODO: send lifecycle notification
    }

    /// Called when a document's syntax definition has changed.
    pub fn document_syntax_changed(&mut self, view_id: &ViewIdentifier) {
        print_err!("document_syntax_changed {}", view_id);
        let buffer_id = self.lock().buffer_for_view(view_id);

        let start_keys = self.activatable_plugins(view_id)
            .iter()
            .map(|plg| (buffer_id.clone(), plg.to_owned()))
            .collect::<BTreeSet<_>>();

        // stop currently running plugins that aren't on list
        // TODO: don't stop plugins that weren't started by a syntax activation
        let to_stop = self.lock().running.keys()
            .filter(|k| !start_keys.contains(k))
            .map(|k| k.1.to_owned())
            .collect::<Vec<_>>();

        let to_run = start_keys.iter()
            .filter(|k| !self.lock().running.contains_key(&k))
            .map(|k| k.1.to_owned())
            .collect::<Vec<_>>();

        for plugin_name in to_stop {
            self.stop_plugin(&view_id, &plugin_name);
        }

        self.start_plugins(view_id, &to_run);
    }

    /// Launches and initializes the named plugin.
    pub fn start_plugin(&self, view_id: &ViewIdentifier, plugin_name: &str,
                        init_info: &PluginBufferInfo) {
        self.lock().start_plugin(self, view_id, plugin_name, init_info.to_owned());
    }

    /// Terminates and cleans up the named plugin.
    pub fn stop_plugin(&self, view_id: &ViewIdentifier, plugin_name: &str) {
        self.lock().stop_plugin(view_id, plugin_name);
    }

    /// Forward an update from a view to registered plugins.
    pub fn update_plugins(&self, view_id: &ViewIdentifier,
                          update: PluginUpdate, undo_group: usize) {
        self.lock().update(view_id, update, undo_group)
    }

    // ====================================================================
    // implementation details
    // ====================================================================

    /// Returns the plugins which want to activate for this view.
    ///
    /// That a plugin wants to activate does not mean it will be activated.
    /// For instance, it could have already been disabled by user preference.
    fn activatable_plugins(&self, view_id: &ViewIdentifier) -> Vec<String> {
        let inner = self.lock();
        let syntax = inner.buffers.lock()
            .editor_for_view(view_id).unwrap().get_syntax().to_owned();

        inner.catalog.iter()
            .filter(|plug_desc|{
                plug_desc.activations.iter().any(|act|{
                    match *act {
                        PluginActivation::Autorun => true,
                        PluginActivation::OnSyntax(ref other) if *other == syntax => true,
                        _ => false,
                    }
                })
            })
        .map(|desc| desc.name.to_owned())
            .collect::<Vec<_>>()
    }

    /// Batch run a group of plugins (as on creating a new view, for instance)
    fn start_plugins(&mut self, view_id: &ViewIdentifier, plugin_names: &Vec<String>) {
        print_err!("starting plugins for {}", view_id);

        let init_info = {
            let inner = self.lock();
            let init_info = inner.buffers.lock().editor_for_view(view_id)
                .unwrap().plugin_init_info();
            init_info
        };

        for plugin_name in plugin_names.iter() {
            print_err!("starting plugin {}", plugin_name);
            self.start_plugin(view_id, plugin_name, &init_info);
        }
    }
}
