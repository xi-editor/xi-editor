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

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::{Arc, Mutex, Weak, MutexGuard};

use serde_json::{self, Value};

use tabs::{BufferIdentifier, ViewIdentifier, BufferContainerRef};

use super::{PluginDescription, PluginRef};
use super::rpc_types::{PluginCommand, PluginUpdate, UpdateResponse};
use super::manifest::debug_plugins;

type PluginName = String;

/// Manages plugin loading, activation, lifecycle, and dispatch.
pub struct PluginManager<W: Write> {
    catalog: Vec<PluginDescription>,
    running: BTreeMap<(BufferIdentifier, PluginName), PluginRef<W>>,
    buffers: BufferContainerRef<W>,
}

impl <W: Write>PluginManager<W> {
    pub fn debug_available_plugins(&self) -> Vec<&str> {
        self.catalog.iter().map(|p| p.name.as_ref()).collect::<Vec<_>>()
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
        print_err!("document_new {}", view_id);
    }

    /// Called when an existing file is loaded into a buffer.
    pub fn document_open(&mut self, view_id: &ViewIdentifier) {
        print_err!("document_open {}", view_id);
    }

    /// Called when a buffer is saved to a file.
    pub fn document_did_save(&mut self, view_id: &ViewIdentifier) {
        print_err!("document_did_save {}", view_id);
    }

    /// Called when a buffer is closed.
    pub fn document_close(&mut self, view_id: &ViewIdentifier) {
        print_err!("document_close {}", view_id);
    }

    /// Called when a document's syntax definition has changed.
    pub fn document_syntax_changed(&mut self, view_id: &ViewIdentifier) {
        print_err!("document_syntax_changed {}", view_id);
    }

    /// Passes an update from a buffer to all registered plugins.
    pub fn update(&mut self, view_id: &ViewIdentifier, update: PluginUpdate,
                  undo_group: usize) {
        // find all running plugins for this buffer, and send them the update
        let inner = self.lock();
        for (_, plugin) in inner.running.iter().filter(|kv| (kv.0).0 == *view_id) {
            inner.buffers.lock().editor_for_view_mut(view_id)
                .unwrap().increment_revs_in_flight();
            let view_id = view_id.to_owned();
            let buffers = inner.buffers.clone().to_weak();
            plugin.update(&update, move |response| {
                if let Some(buffers) = buffers.upgrade() {
                    let response = response.expect("bad plugin response");
                    match serde_json::from_value::<UpdateResponse>(response) {
                        Ok(UpdateResponse::Edit(edit)) => {
                            buffers.lock().editor_for_view_mut(&view_id).unwrap()
                                .apply_plugin_edit(edit, undo_group);
                        }
                        Ok(UpdateResponse::Ack(_)) => (),
                        Err(err) => { print_err!("plugin response json err: {:?}", err); }
                    }
                    buffers.lock().editor_for_view_mut(&view_id)
                        .unwrap().dec_revs_in_flight();
                }
            })
        }
        inner.buffers.lock().editor_for_view_mut(view_id).unwrap().dec_revs_in_flight();
    }

    /// Launches and initializes the named plugin.
    pub fn start_plugin(&mut self, buffer_id: &str, plugin_name: &str,
                        buf_size: usize, rev: usize) {
        //TODO: error handling: this should maybe have a completion callback with a Result
        let key = (buffer_id.to_owned(), plugin_name.to_owned());
        let inner = self.lock();
        if inner.running.contains_key(&key) {
            print_err!("plugin {} already running for buffer {}", plugin_name, buffer_id);
        }

        let plugin = inner.catalog.iter().find(|desc| desc.name == plugin_name)
            .expect(&format!("no plugin found with name {}", plugin_name));

        let me = self.clone();
        plugin.launch(self, buffer_id, move |result| {
            match result {
                Ok(plugin_ref) => {
                    plugin_ref.init_buf(buf_size, rev);
                    me.lock().running.insert(key, plugin_ref);
                },
                Err(_) => panic!("error handling is not implemented"),
            }
        });
    }

    /// Handle a request from a plugin.
    pub fn handle_plugin_cmd(&self, cmd: PluginCommand, view_id: &ViewIdentifier) -> Option<Value> {
        use self::PluginCommand::*;
        let inner = self.lock();
        match cmd {
            LineCount => {
                let n_lines = inner.buffers.lock().editor_for_view(view_id).unwrap()
                    .plugin_n_lines() as u64;
                Some(serde_json::to_value(n_lines).unwrap())
            },
            SetFgSpans { start, len, spans, rev } => {
                inner.buffers.lock().editor_for_view_mut(view_id).unwrap()
                    .plugin_set_fg_spans(start, len, spans, rev);
                None
            }
            GetData { offset, max_size, rev } => {
                inner.buffers.lock().editor_for_view(view_id).unwrap()
                .plugin_get_data(offset, max_size, rev)
                .map(|data| Value::String(data))
            }

            Alert { msg } => {
                inner.buffers.lock().editor_for_view(view_id).unwrap()
                .plugin_alert(&msg);
                None
            }
        }
    }
}

