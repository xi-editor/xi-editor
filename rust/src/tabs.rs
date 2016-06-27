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

use xi_rope::rope::Rope;
use editor::Editor;
use rpc::{send, TabCommand, EditCommand};

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

    pub fn do_rpc(&mut self, cmd: TabCommand) -> Option<Value> {
        use rpc::TabCommand::*;

        match cmd {
            NewTab => Some(Value::String(self.do_new_tab())),

            DeleteTab { tab_name } => {
                self.do_delete_tab(tab_name);
                None
            },

            Edit { tab_name, edit_command } => self.do_edit(tab_name, edit_command),
        }
    }

    fn do_new_tab(&mut self) -> String {
        self.new_tab()
    }

    fn do_delete_tab(&mut self, tab: &str) {
        self.delete_tab(tab);
    }

    fn do_edit(&mut self, tab: &str, cmd: EditCommand) -> Option<Value> {
        if let Some(editor) = self.tabs.get_mut(tab) {
            editor.do_rpc(cmd, &self.kill_ring)
        } else {
            print_err!("tab not found: {}", tab);
            None
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
        .insert_object("params", |builder| {
            builder.insert("tab", tab)
                .insert("update", update)
        })
        .unwrap()) {
        print_err!("send error on update_tab: {}", e);
    }
}
