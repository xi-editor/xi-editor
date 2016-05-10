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

//! A container for all the tabs being edited. Also functions as main dispatch for RPC.

use std::collections::BTreeMap;
use std::sync::Mutex;
use serde_json::Value;
use serde_json::builder::ObjectBuilder;
use serde::ser::Serialize;

use xi_rope::rope::Rope;
use editor::Editor;
use ::send;

pub struct Tabs {
    tabs: BTreeMap<String, Editor>,
    id_counter: usize,
    kill_ring: Mutex<Rope>,
}

impl Tabs {
    pub fn new() -> Tabs {
        Tabs {
            tabs: BTreeMap::new(),
            id_counter: 0,
            kill_ring: Mutex::new(Rope::from("")),
        }
    }

    // TODO: refactor response in here, rather than explicitly calling "respond"
    pub fn handle_rpc(&mut self, method: &str, params: &Value, id: Option<&Value>) {
        match method {
            "new_tab" => self.do_new_tab(id),
            "delete_tab" => self.do_delete_tab(params),
            "edit" => self.do_edit(params, id),
            _ => print_err!("unknown method {}", method),
        }
    }

    pub fn respond<V>(&self, result: V, id: Option<&Value>)
            where V: Serialize {
        if let Some(id) = id {
            if let Err(e) = send(&ObjectBuilder::new()
                .insert("id", id)
                .insert("result", result)
                .unwrap()
            ) {
                print_err!("error {} sending response to RPC {:?}", e, id);
            }
        } else {
            print_err!("tried to respond with no id");
        }
    }

    fn do_new_tab(&mut self, id: Option<&Value>) {
        let tabname = self.new_tab();
        self.respond(&tabname, id);
    }

    fn do_delete_tab(&mut self, params: &Value) {
        if let Some(params) = params.as_object() {
            let tab = params.get("tab").unwrap().as_string().unwrap();
            self.delete_tab(tab);
        }
    }

    fn do_edit(&mut self, params: &Value, id: Option<&Value>) {
        if let Some(params) = params.as_object() {
            let tab = params.get("tab").unwrap().as_string().unwrap();
            let response = {
                if let Some(editor) = self.tabs.get_mut(tab) {
                    let method = params.get("method").unwrap().as_string().unwrap();
                    let params = params.get("params").unwrap();
                    editor.do_rpc(method, params, &self.kill_ring)
                } else {
                    print_err!("tab not found: {}", tab);
                    None
                }
            };
            if let Some(response) = response {
                self.respond(response, id);
            } else if let Some(id) = id {
                print_err!("rpc with id={:?} not responded", id);
            }
        }
    }

    fn new_tab(&mut self) -> String {
        let tabname = self.id_counter.to_string();
        self.id_counter += 1;
        let editor = Editor::new(&tabname);
        self.tabs.insert(tabname.clone(), editor);
        tabname
    }

    fn delete_tab(&mut self, tabname: &str) {
        self.tabs.remove(tabname);
    }
}

// arguably this should be a method on a newtype for tab. but keep things simple for now
pub fn update_tab(update: &Value, tab: &str) {
    if let Err(e) = send(&ObjectBuilder::new()
        .insert("method", "update")
        .insert_object("params", |builder|
            builder.insert("tab", tab)
                .insert("update", update))
        .unwrap()
    ) {
        print_err!("send error on update_tab: {}", e);
    }
}
