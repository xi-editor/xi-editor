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
use std::sync::{Arc, Mutex};

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


impl<W: Write + Send + 'static> PluginManager<W> {
    pub fn new(buffers: BufferContainerRef<W>) -> Self {
        PluginManager {
            // TODO: actually parse these from manifest files
            catalog: debug_plugins(),
            running: BTreeMap::new(),
            buffers: buffers,
        }
    }

    pub fn debug_available_plugins(&self) -> Vec<&str> {
        self.catalog.iter().map(|p| p.name.as_ref()).collect::<Vec<_>>()
    }

    //fn new_buffer(&mut self, view_id: &ViewIdentifier) {}
    //fn file_open(&mut self, view_id: &ViewIdentifier, path: &Path) {}
    //fn file_save(&mut self, view_id: &ViewIdentifier, path: &Path) {}
    //fn view_close(&mut self, view_id: &ViewIdentifier) {}

    /// Passes an update from a buffer to all registered plugins.
    pub fn update(&mut self, view_id: &ViewIdentifier, update: PluginUpdate,
                  undo_group: usize) {
        // find all running plugins for this buffer, and send them the update
        for (_, plugin) in self.running.iter().filter(|kv| (kv.0).0 == *view_id) {
            self.buffers.lock().editor_for_view_mut(view_id)
                .unwrap().increment_revs_in_flight();
            let view_id = view_id.to_owned();
            let buffers = self.buffers.clone().to_weak();
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
        self.buffers.lock().editor_for_view_mut(view_id).unwrap().dec_revs_in_flight();
    }

    /// Launches and initializes the named plugin.
    pub fn start_plugin(&mut self, self_ref: &Arc<Mutex<PluginManager<W>>>,
                          buffer_id: &str, plugin_name: &str, buf_size: usize, rev: usize) {
        //TODO: error handling: this should maybe have a completion callback with a Result
        let key = (buffer_id.to_owned(), plugin_name.to_owned());
        if self.running.contains_key(&key) {
            print_err!("plugin {} already running for buffer {}", plugin_name, buffer_id);
        }
        let plugin = self.catalog.iter().find(|desc| desc.name == plugin_name)
            .expect(&format!("no plugin found with name {}", plugin_name));

        let me = self_ref.clone();
        plugin.launch(self_ref, buffer_id, move |result| {
            match result {
                Ok(plugin_ref) => {
                    plugin_ref.init_buf(buf_size, rev);
                    me.lock().unwrap().running.insert(key, plugin_ref);
                },
                Err(_) => panic!("error handling is not implemented"),
            }
        });
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
}
