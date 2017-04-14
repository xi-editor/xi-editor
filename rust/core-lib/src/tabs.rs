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
use std::io::{self, Read, Write};
use std::path::Path;
use std::fs::File;
use std::sync::{Arc, Mutex};
use serde_json::value::Value;

use xi_rope::rope::Rope;
use editor::Editor;
use rpc::{TabCommand, EditCommand};
use styles::{Style, StyleMap};
use MainPeer;

/// ViewIdentifiers are the primary means of routing messages between xi-core and a client view.
pub type ViewIdentifier = String;

// NOTE: there's a case that this should be an enum, with PathBuf & String members (for real files &
//unsaved buffers)
/// BufferIdentifiers are placeholder names used to identify buffers that do not have a filename.
type BufferIdentifier = String;

//TODO: proposed new name: something like "Core" or "CoreState" or "EditorState"?
pub struct Tabs<W: Write> {
    // NOTE: this is to allow multiple views into a single buffer
    /// maps buffer identifiers (filenames) to editor instances
    editors: BTreeMap<BufferIdentifier, Arc<Mutex<Editor<W>>>>,
    /// maps view identifiers to editor instances. All actions originate in a view; this lets us
    /// route messages correctly when multiple views share a buffer.
    views: BTreeMap<ViewIdentifier, Arc<Mutex<Editor<W>>>>,
    id_counter: usize,
    kill_ring: Arc<Mutex<Rope>>,
    style_map: Arc<Mutex<StyleMap>>,
}

#[derive(Clone)]
pub struct TabCtx<W: Write> {
    // NOTE: this is essentially the buffer path. Should this be saved here, or in the editor
    //itself, or does it matter?
    //buffer_id: String,
    tab: ViewIdentifier,
    kill_ring: Arc<Mutex<Rope>>,
    rpc_peer: MainPeer<W>,
    style_map: Arc<Mutex<StyleMap>>,
}


impl<W: Write + Send + 'static> Tabs<W> {
    pub fn new() -> Tabs<W> {
        Tabs {
            editors: BTreeMap::new(),
            views: BTreeMap::new(),
            id_counter: 0,
            kill_ring: Arc::new(Mutex::new(Rope::from(""))),
            style_map: Arc::new(Mutex::new(StyleMap::new())),
        }
    }

    fn new_tab_ctx(&self, buffer_id: &str, peer: &MainPeer<W>) -> TabCtx<W> {
        TabCtx {
            tab: buffer_id.to_owned(),
            kill_ring: self.kill_ring.clone(),
            rpc_peer: peer.clone(),
            style_map: self.style_map.clone(),
        }
    }

    fn next_view_id(&mut self) -> ViewIdentifier {
        self.id_counter += 1;
        format!("view-id-{}", self.id_counter)
    }

    fn next_buffer_id(&mut self) -> BufferIdentifier {
        self.id_counter += 1;
        format!("buffer-id-{}", self.id_counter)
    }

    pub fn do_rpc(&mut self, cmd: TabCommand, rpc_peer: &MainPeer<W>) -> Option<Value> {
        use rpc::TabCommand::*;

        match cmd {
            CloseView { view_id } => {
                self.do_close_view(view_id);
                None
            },

            NewView { file_path } => Some(Value::String(self.do_new_view(rpc_peer, file_path))),

            //TODO: intercept save, to make make sure we keep buffer_id up to date?
            Edit { tab_name, edit_command } => self.do_edit(tab_name, edit_command),
            _ => {  print_err!("unsupported command {:?}.", cmd); None },
        }
    }

    fn do_new_view(&mut self, rpc_peer: &MainPeer<W>, file_path: Option<&str>) -> String {
        // currently two code paths (new buffer, open file) but will eventually be at least three
        // (new view into existing buffer)
        if let Some(file_path) = file_path {
            self.new_view_with_file(rpc_peer, file_path)
        }  else {
            self.new_empty_view(rpc_peer)
        }
    }

    fn do_close_view(&mut self, view_id: &str) {
        self.close_view(view_id);
    }

    fn do_edit(&mut self, view_id: &str, cmd: EditCommand)
            -> Option<Value> {
        if let Some(editor) = self.views.get(view_id) {
            Editor::do_rpc(editor, cmd)
        } else {
            print_err!("tab not found: {}", view_id);
            None
        }
    }

    fn new_empty_view(&mut self, rpc_peer: &MainPeer<W>) -> String {
        let view_id = self.next_view_id();
        let buffer_id = self.next_buffer_id();
        let editor = Editor::new(self.new_tab_ctx(&view_id, rpc_peer));
        self.finalize_new_view(view_id, buffer_id, editor)
    }

    fn new_view_with_file(&mut self, rpc_peer: &MainPeer<W>, file_path:&str) -> String {
        //TODO: double check logic around buffer_id / refactor this to be more readable
        //TODO: there's a good argument that we should be validating/opening file_path here, so we
        //can report errors to the client
        match self.read_file(file_path) {
            Ok(contents) => {
                let view_id = self.next_view_id();
                let buffer_id = file_path.to_owned();
                let editor = Editor::with_text(self.new_tab_ctx(&view_id, rpc_peer), contents);
                self.finalize_new_view(view_id, buffer_id, editor)
            }
            Err(err) => {
                //TODO: we should be reporting errors to the client
                print_err!("unable to read file: {}, error: {:?}", file_path, err);
               self.new_empty_view(rpc_peer)
            }
        }
    }

    fn finalize_new_view(&mut self, view_id: String, buffer_id: String, editor: Arc<Mutex<Editor<W>>>) -> String {

        editor.lock().unwrap().add_view(view_id.clone());
        self.views.insert(view_id.clone(), editor.clone());
        //TODO: stash buffer_id + editor
        //self.editors.insert(buffer_id, editor);
        view_id
    }
    
    fn read_file<P: AsRef<Path>>(&self, path: P) -> io::Result<String> {
        let mut f = File::open(path)?;
        let mut s = String::new();
        f.read_to_string(&mut s)?;
        Ok(s)
    }
    
    fn close_view(&mut self, view_id: &str) {
        //TODO: this should obviously also be removing from the buffers list if it's the last
        //reference
        let ed = self.views.remove(view_id).expect("missing editor when closing view");
        //let buffer_id = ed.lock().unwrap().tab_ctx
    }

    pub fn handle_idle(&self) {
        for editor in self.editors.values() {
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
