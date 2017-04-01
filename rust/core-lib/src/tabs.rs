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
use std::io::Write;
use std::sync::{Arc, Mutex};
use serde_json::value::Value;

use xi_rope::rope::Rope;
use editor::Editor;
use rpc::{TabCommand, EditCommand};
use styles::{Style, StyleMap};
use MainPeer;

pub struct Tabs<W: Write> {
    tabs: BTreeMap<String, Arc<Mutex<Editor<W>>>>,
    id_counter: usize,
    kill_ring: Arc<Mutex<Rope>>,
    style_map: Arc<Mutex<StyleMap>>,
}

#[derive(Clone)]
pub struct TabCtx<W: Write> {
    tab: String,
    kill_ring: Arc<Mutex<Rope>>,
    rpc_peer: MainPeer<W>,
    style_map: Arc<Mutex<StyleMap>>,
}

impl<W: Write + Send + 'static> Tabs<W> {
    pub fn new() -> Tabs<W> {
        Tabs {
            tabs: BTreeMap::new(),
            id_counter: 0,
            kill_ring: Arc::new(Mutex::new(Rope::from(""))),
            style_map: Arc::new(Mutex::new(StyleMap::new())),
        }
    }

    pub fn do_rpc(&mut self, cmd: TabCommand, rpc_peer: &MainPeer<W>) -> Option<Value> {
        use rpc::TabCommand::*;

        match cmd {
            NewTab => Some(Value::String(self.do_new_tab(rpc_peer))),

            DeleteTab { tab_name } => {
                self.do_delete_tab(tab_name);
                None
            },

            Edit { tab_name, edit_command } => self.do_edit(tab_name, edit_command),
        }
    }

    fn do_new_tab(&mut self, rpc_peer: &MainPeer<W>) -> String {
        self.new_tab(rpc_peer)
    }

    fn do_delete_tab(&mut self, tab: &str) {
        self.delete_tab(tab);
    }

    fn do_edit(&mut self, tab: &str, cmd: EditCommand)
            -> Option<Value> {
        if let Some(editor) = self.tabs.get(tab) {
            Editor::do_rpc(editor, cmd)
        } else {
            print_err!("tab not found: {}", tab);
            None
        }
    }

    fn new_tab(&mut self, rpc_peer: &MainPeer<W>) -> String {
        let tabname = self.id_counter.to_string();
        self.id_counter += 1;
        let tab_ctx = TabCtx {
            tab: tabname.clone(),
            kill_ring: self.kill_ring.clone(),
            rpc_peer: rpc_peer.clone(),
            style_map: self.style_map.clone(),
        };
        let editor = Editor::new(tab_ctx);
        self.tabs.insert(tabname.clone(), editor);
        tabname
    }

    fn delete_tab(&mut self, tabname: &str) {
        self.tabs.remove(tabname);
    }

    pub fn handle_idle(&self) {
        for editor in self.tabs.values() {
            editor.lock().unwrap().render();
        }
    }
}

impl<W: Write> TabCtx<W> {
    pub fn update_tab(&self, update: &Value) {
        self.rpc_peer.send_rpc_notification("update",
            &json!({
                "tab": &self.tab,
                "update": update,
            }));
    }

    pub fn scroll_to(&self, line: usize, col: usize) {
        self.rpc_peer.send_rpc_notification("scroll_to",
            &json!({
                "tab": &self.tab,
                "line": line,
                "col": col,
            }));
    }

    pub fn get_kill_ring(&self) -> Rope {
        self.kill_ring.lock().unwrap().clone()
    }

    pub fn set_kill_ring(&self, val: Rope) {
        let mut kill_ring = self.kill_ring.lock().unwrap();
        *kill_ring = val;
    }

    pub fn alert(&self, msg: &str) {
        self.rpc_peer.send_rpc_notification("alert",
            &json!({
                "msg": msg,
            }));
    }

    // Get the index for a given style. If the style is not in the existing
    // style map, then issues a def_style request to the front end. Intended
    // to be reasonably efficient, but ideally callers would do their own
    // indexing.
    pub fn get_style_id(&self, style: &Style) -> usize {
        let mut style_map = self.style_map.lock().unwrap();
        if let Some(ix) = style_map.lookup(style) {
            return ix;
        }
        let ix = style_map.add(style);
        self.rpc_peer.send_rpc_notification("def_style", &style.to_json(ix));
        ix
    }
}
