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
use std::io::{self, Write};
use std::sync::{Arc, Mutex, Weak, MutexGuard};

use serde_json::{self, Value};

use tabs::{BufferIdentifier, ViewIdentifier, BufferContainerRef};

use super::{PluginDescription, PluginRef, start_plugin_process, PluginPid};
use super::rpc_types::{PluginCommand, PluginUpdate, UpdateResponse, ClientPluginInfo, PluginBufferInfo};
use super::manifest::{PluginActivation, debug_plugins};

type PluginName = String;
type BufferPlugins<W> = BTreeMap<PluginName, PluginRef<W>>;

/// Manages plugin loading, activation, lifecycle, and dispatch.
pub struct PluginManager<W: Write> {
    catalog: Vec<PluginDescription>,
    running: BTreeMap<BufferIdentifier, BufferPlugins<W>>,
    buffers: BufferContainerRef<W>,
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

impl <W: Write + Send + 'static>PluginManager<W> {

    /// Returns plugins available to this view.
    pub fn available_plugins(&self, view_id: &ViewIdentifier) -> Vec<ClientPluginInfo> {
        self.catalog.iter().map(|p| {
            let running = self.running_for_view(view_id)
                .map(|plugins| plugins.contains_key(&p.name))
                .unwrap_or_default();
            let name = p.name.clone();
            ClientPluginInfo { name, running }
        }).collect::<Vec<_>>()
    }

    /// Handle a request from a plugin.
    pub fn handle_plugin_cmd(&self, cmd: PluginCommand, view_id: &ViewIdentifier) -> Option<Value> {
        use self::PluginCommand::*;
        match cmd {
            LineCount => {
                let n_lines = self.buffers.lock().editor_for_view(view_id).unwrap()
                    .plugin_n_lines() as u64;
                Some(serde_json::to_value(n_lines).unwrap())
            },
            SetFgSpans { start, len, spans, rev } => {
                self.buffers.lock().editor_for_view_mut(view_id).unwrap()
                    .plugin_set_fg_spans(start, len, spans, rev);
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
    fn update_plugins(&mut self, view_id: &ViewIdentifier,
                  update: PluginUpdate, undo_group: usize) -> Result<(), Error> {

        // find all running plugins for this buffer, and send them the update
        let mut dead_plugins = Vec::new();

        if let Ok(running) = self.running_for_view(view_id) {
            for (name, plugin) in running.iter() {
                // check to see if plugins have crashed
                if plugin.is_dead() {
                    dead_plugins.push(name.to_owned());
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
                });
            }
        };
        self.cleanup_dead(view_id, &dead_plugins);
        self.buffers.lock().editor_for_view_mut(view_id).unwrap().dec_revs_in_flight();
        Ok(())
    }

    /// Launches and initializes the named plugin.
    fn start_plugin(&mut self, self_ref: &PluginManagerRef<W>,
                    view_id: &ViewIdentifier, plugin_name: &str,
                    init_info: PluginBufferInfo) -> Result<(), Error> {

        let _ = match self.running_for_view(view_id) {
            Ok(plugins) if plugins.contains_key(plugin_name) => {
                Err(Error::Other(format!("{} already running", plugin_name)))
            }
            Err(err) => Err(err),
            Ok(_) => Ok(()),
        }?;

        let plugin_id = self.next_plugin_id();
        let plugin = self.catalog.iter()
            .find(|desc| desc.name == plugin_name)
            .ok_or(Error::Other(format!("no plugin found with name {}", plugin_name)))?;

        let me = self_ref.clone();
        let view_id2 = view_id.to_owned();
        let plugin_name = plugin_name.to_owned();

        start_plugin_process(self_ref, &plugin, plugin_id, &view_id, move |result| {
            let view_id = view_id2;
            match result {
                Ok(plugin_ref) => {
                    plugin_ref.initialize(&init_info);
                    let mut inner = me.lock();
                    let mut running = false;

                    if let Some(ed) = inner.buffers.lock().editor_for_view(&view_id) {
                        ed.plugin_started(&view_id, &plugin_name);
                        running = true;
                    }
                    if running {
                        let _ = inner.running_for_view_mut(&view_id).map(|running| {
                            running.insert(plugin_name, plugin_ref);
                        });
                    }
                }
                Err(_) => print_err!("failed to start plugin {}", plugin_name),
            }
        });
        Ok(())
    }

    fn stop_plugin(&mut self, view_id: &ViewIdentifier, plugin_name: &str) {
        let plugin_ref = match self.running_for_view_mut(view_id) {
            Ok(running) => running.remove(plugin_name),
            Err(_) => None,
        };

        if let Some(plugin_ref) = plugin_ref {
            plugin_ref.shutdown();
            //TODO: should we notify now, or wait until we know this worked?
            //can this fail? (yes.) How do we tell, and when do we kill the proc?
            if let Some(editor) = self.buffers.lock().editor_for_view(view_id) {
                editor.plugin_stopped(view_id, plugin_name, 0);
            }
        }
    }

    /// Remove dead plugins, notifying editors as needed.
    //TODO: this currently only runs after trying to update a plugin that has crashed
    // during a previous update: that is, if a plugin crashes it isn't cleaned up
    // immediately. If this is a problem, we should store crashes, and clean up in idle().
    fn cleanup_dead(&mut self, view_id: &ViewIdentifier, plugins: &[PluginName]) {
        for name in plugins.iter() {
            let _ = self.running_for_view_mut(&view_id)
                .map(|running| running.remove(name));
            self.buffers.lock().editor_for_view(view_id).map(|ed|{
                //TODO: define exit codes, put them in an enum somewhere
                let abnormal_exit_code = 1;
                ed.plugin_stopped(&view_id, &name, abnormal_exit_code);
            });
        }
    }

    // ====================================================================
    // convenience functions
    // ====================================================================

    fn buffer_for_view(&self, view_id: &ViewIdentifier) -> Option<BufferIdentifier> {
        self.buffers.buffer_for_view(view_id).map(|id| id.to_owned())
    }

    fn next_plugin_id(&mut self) -> PluginPid {
        self.next_id += 1;
        PluginPid(self.next_id)
    }

    fn plugin_is_running(&self, view_id: &ViewIdentifier, plugin_name: &PluginName) -> bool {
        self.buffer_for_view(view_id)
            .and_then(|id| self.running.get(&id))
            .map(|plugins| plugins.contains_key(plugin_name))
            .unwrap_or_default()
    }

    fn running_for_view(&self, view_id: &ViewIdentifier) -> Result<&BufferPlugins<W>, Error> {
        self.buffer_for_view(view_id)
            .and_then(|id| self.running.get(&id))
            .ok_or(Error::EditorMissing)
    }

    fn running_for_view_mut(&mut self, view_id: &ViewIdentifier) -> Result<&mut BufferPlugins<W>, Error> {
        let buffer_id = match self.buffer_for_view(view_id) {
            Some(id) => Ok(id),
            None => Err(Error::EditorMissing),
        }?;
        self.running.get_mut(&buffer_id)
            .ok_or(Error::EditorMissing)
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
            next_id: 0,
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
        self.add_running_collection(view_id);
        let to_start = self.activatable_plugins(view_id);
        self.start_plugins(view_id, &to_start);
        //TODO: send lifecycle notification
    }

    /// Called when an existing file is loaded into a buffer.
    pub fn document_open(&mut self, view_id: &ViewIdentifier) {
        print_err!("document open {}", view_id);
        self.add_running_collection(view_id);
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
        //TODO: send lifecycle notification
    }

    /// Called when a document's syntax definition has changed.
    pub fn document_syntax_changed(&mut self, view_id: &ViewIdentifier) {
        print_err!("document_syntax_changed {}", view_id);

        let start_keys = self.activatable_plugins(view_id).iter()
            .map(|p| p.to_owned())
            .collect::<BTreeSet<_>>();

        // stop currently running plugins that aren't on list
        // TODO: don't stop plugins that weren't started by a syntax activation
        let to_stop = self.lock().running_for_view(view_id)
            .unwrap().keys()
            .filter(|k| !start_keys.contains(*k))
            .map(|k| k.to_owned())
            .collect::<Vec<String>>();

        let to_run = start_keys.iter()
            .filter(|k| !self.lock().plugin_is_running(view_id, k))
            .map(|k| k.clone().to_owned())
            .collect::<Vec<String>>();

        for plugin_name in to_stop {
            self.stop_plugin(&view_id, &plugin_name);
        }
        self.start_plugins(view_id, &to_run);
    }

    /// Launches and initializes the named plugin.
    pub fn start_plugin(&self, view_id: &ViewIdentifier, plugin_name: &str,
                        init_info: &PluginBufferInfo) -> Result<(), Error> {
        self.lock().start_plugin(self, view_id, plugin_name, init_info.to_owned())
    }

    /// Terminates and cleans up the named plugin.
    pub fn stop_plugin(&self, view_id: &ViewIdentifier, plugin_name: &str) {
        self.lock().stop_plugin(view_id, plugin_name);
    }

    /// Forward an update from a view to registered plugins.
    pub fn update_plugins(&self, view_id: &ViewIdentifier,
                          update: PluginUpdate, undo_group: usize) -> Result<(), Error> {
        self.lock().update_plugins(view_id, update, undo_group)
    }

    // ====================================================================
    // implementation details
    // ====================================================================

    /// Performs new buffer setup
    fn add_running_collection(&self, view_id: &ViewIdentifier) {
        assert!(self.lock().running_for_view(view_id).is_err());
        let buffer_id = self.lock().buffer_for_view(view_id)
            .expect("document new expects buffer");
        self.lock().running.insert(buffer_id, BufferPlugins::new());
    }

    /// Returns the plugins which want to activate for this view.
    ///
    /// That a plugin wants to activate does not mean it will be activated.
    /// For instance, it could have already been disabled by user preference.
    fn activatable_plugins(&self, view_id: &ViewIdentifier) -> Vec<String> {
        let inner = self.lock();
        let syntax = inner.buffers.lock()
            .editor_for_view(view_id)
            .unwrap()
            .get_syntax()
            .to_owned();

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
            let init_info = inner.buffers.lock()
                .editor_for_view(view_id)
                .map(|ed| ed.plugin_init_info());
            init_info
        };

        if let Some(init_info) = init_info {
            for plugin_name in plugin_names.iter() {
                match self.start_plugin(view_id, &plugin_name, &init_info) {
                    Ok(_) => print_err!("starting plugin {}", &plugin_name),
                    Err(err) => print_err!("unable to start plugin {}, err: {:?}",
                                           &plugin_name, err),
                }
            }
        } else {
            print_err!("no editor for view {}", view_id)
        }
    }
}
