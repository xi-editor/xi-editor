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
use std::path::{PathBuf, Path};
use std::fs::File;
use std::sync::{Arc, Mutex};
use serde_json::value::Value;

use xi_rope::rope::Rope;
use editor::Editor;
use rpc::{CoreCommand, EditCommand};
use styles::{Style, StyleMap};
use MainPeer;

/// ViewIdentifiers are the primary means of routing messages between xi-core and a client view.
pub type ViewIdentifier = String;

/// BufferIdentifiers uniquely identify open buffers.
type BufferIdentifier = String;

// TODO: proposed new name: something like "Core" or "CoreState" or "EditorState"? "Documents?"
pub struct Tabs<W: Write> {
    /// maps file names to buffer identifiers. If a client asks to open a file that is already
    /// open, we treat it as a request for a new view.
    open_files: BTreeMap<PathBuf, BufferIdentifier>,
    /// maps buffer identifiers (filenames) to editor instances
    buffers: BTreeMap<BufferIdentifier, Arc<Mutex<Editor<W>>>>,
    /// maps view identifiers to editor instances. All actions originate in a view; this lets us
    /// route messages correctly when multiple views share a buffer.
    views: BTreeMap<ViewIdentifier, BufferIdentifier>,
    id_counter: usize,
    kill_ring: Arc<Mutex<Rope>>,
    style_map: Arc<Mutex<StyleMap>>,
}

#[derive(Clone)]
pub struct TabCtx<W: Write> {
    kill_ring: Arc<Mutex<Rope>>,
    rpc_peer: MainPeer<W>,
    style_map: Arc<Mutex<StyleMap>>,
}


impl<W: Write + Send + 'static> Tabs<W> {
    pub fn new() -> Tabs<W> {
        Tabs {
            open_files: BTreeMap::new(),
            buffers: BTreeMap::new(),
            views: BTreeMap::new(),
            id_counter: 0,
            kill_ring: Arc::new(Mutex::new(Rope::from(""))),
            style_map: Arc::new(Mutex::new(StyleMap::new())),
        }
    }

    fn new_tab_ctx(&self, peer: &MainPeer<W>) -> TabCtx<W> {
        TabCtx {
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

    pub fn do_rpc(&mut self, cmd: CoreCommand, rpc_peer: &MainPeer<W>) -> Option<Value> {
        use rpc::CoreCommand::*;

        match cmd {
            CloseView { view_id } => {
                self.do_close_view(view_id);
                None
            },

            NewView { file_path } => Some(Value::String(self.do_new_view(rpc_peer, file_path))),
            Save { view_id, file_path } => self.do_save(view_id, file_path),
            Edit { view_id, edit_command } => self.do_edit(view_id, edit_command),
        }
    }

    /// Creates a new view and associates it with a buffer.
    ///
    /// This function always creates a new view and associates it with a buffer (which we access
    ///through an `Editor` instance). This buffer may be existing, or it may be created.
    ///
    ///A `new_view` request is handled differently depending on the `file_path` argument, and on
    ///application state. If `file_path` is given and a buffer associated with that file is already
    ///open, we create a new view into the existing buffer. If `file_path` is given and that file
    ///_isn't_ open, we load that file into a new buffer. If `file_path` is not given, we create a
    ///new empty buffer.
    fn do_new_view(&mut self, rpc_peer: &MainPeer<W>, file_path: Option<&str>) -> ViewIdentifier {
        // three code paths: new buffer, open file, and new view into existing buffer
        let view_id = self.next_view_id();
        if let Some(file_path) = file_path.map(PathBuf::from) {
            // TODO: here, we should eventually be adding views to the existing editor.
            // for the time being, we just create a new empty view.
            if self.open_files.contains_key(&file_path) {
                let buffer_id = self.next_buffer_id();
                self.new_empty_view(rpc_peer, &view_id, buffer_id);
                // let buffer_id = self.open_files.get(&file_path).unwrap().to_owned();
                //self.add_view(&view_id, buffer_id);
            } else {
                // not open: create new buffer_id and open file
                let buffer_id = self.next_buffer_id();
                self.open_files.insert(file_path.to_owned(), buffer_id.clone());
                self.new_view_with_file(rpc_peer, &view_id, buffer_id.clone(), &file_path);
                // above fn has two branches: set path after
                self.buffers.get(&buffer_id).unwrap().lock().unwrap().set_path(&file_path);
            }
        } else {
            // file_path was nil: create a new empty buffer.
            let buffer_id = self.next_buffer_id();
            self.new_empty_view(rpc_peer, &view_id, buffer_id);
        }
        view_id
    }

    fn do_close_view(&mut self, view_id: &str) {
        self.close_view(view_id);
    }

    fn new_empty_view(&mut self, rpc_peer: &MainPeer<W>,
                      view_id: &str, buffer_id: BufferIdentifier) {
        let editor = Editor::new(self.new_tab_ctx(rpc_peer), view_id);
        self.finalize_new_view(view_id, buffer_id, editor);
    }

    fn new_view_with_file(&mut self, rpc_peer: &MainPeer<W>, view_id: &str, buffer_id: BufferIdentifier, path: &Path) {
        match self.read_file(&path) {
            Ok(contents) => {
                let editor = Editor::with_text(self.new_tab_ctx(rpc_peer), view_id, contents);
                self.finalize_new_view(view_id, buffer_id, editor)
            }
            Err(err) => {
                // TODO: we should be reporting errors to the client
                // (if this is even an error? we treat opening a non-existent file as a new buffer,
                // but set the editor's path)
                print_err!("unable to read file: {}, error: {:?}", buffer_id, err);
               self.new_empty_view(rpc_peer, view_id, buffer_id);
            }
        }
    }

    /// Adds a new view to an existing editor instance.
    #[allow(unreachable_code, unused_variables, dead_code)] 
    fn add_view(&mut self, view_id: &str, buffer_id: BufferIdentifier) {
        panic!("add_view should not currently be accessible");
        let editor = self.buffers.get(&buffer_id).expect("missing editor_id for view_id");
        self.views.insert(view_id.to_owned(), buffer_id);
        editor.lock().unwrap().add_view(view_id);
    }

    fn finalize_new_view(&mut self, view_id: &str, buffer_id: String, editor: Arc<Mutex<Editor<W>>>) {
        self.views.insert(view_id.to_owned(), buffer_id.clone());
        self.buffers.insert(buffer_id, editor.clone());
    }
    
    fn read_file<P: AsRef<Path>>(&self, path: P) -> io::Result<String> {
        let mut f = File::open(path)?;
        let mut s = String::new();
        f.read_to_string(&mut s)?;
        Ok(s)
    }
    
    fn close_view(&mut self, view_id: &str) {
        let buf_id = self.views.remove(view_id).expect("missing buffer id when closing view");
        let has_views = {
            let editor = self.buffers.get(&buf_id).expect("missing editor when closing view");
            editor.lock().unwrap().remove_view(view_id);
            editor.lock().unwrap().has_views()
        };

        if !has_views {
            self.buffers.remove(&buf_id);
        }
    }

    fn do_save(&mut self, view_id: &str, file_path: &str) -> Option<Value> {
        let buffer_id = self.views.get(view_id)
            .expect(&format!("missing buffer id for view {}", view_id));
        let editor = self.buffers.get(buffer_id)
            .expect(&format!("missing editor for buffer {}", buffer_id));
        let file_path = PathBuf::from(file_path);

        // if this is a new path for an existing file, we have a bit of housekeeping to do:
        if let Some(prev_path) = editor.lock().unwrap().get_path() {
            if prev_path != file_path {
                self.open_files.remove(prev_path);
            }
        }
        editor.lock().unwrap().do_save(&file_path);
        self.open_files.insert(file_path, buffer_id.to_owned());
        None
    }

    fn do_edit(&mut self, view_id: &str, cmd: EditCommand) -> Option<Value> {
        let buffer_id = self.views.get(view_id)
            .expect(&format!("missing buffer id for view {}", view_id));
        if let Some(editor) = self.buffers.get(buffer_id) {
            Editor::do_rpc(editor, view_id, cmd)
        } else {
            print_err!("buffer not found: {}, for view {}", buffer_id, view_id);
            None
        }
    }

    pub fn handle_idle(&self) {
        for editor in self.buffers.values() {
            editor.lock().unwrap().render();
        }
    }
}

impl<W: Write> TabCtx<W> {
    pub fn update_view(&self, view_id: &str, update: &Value) {
        self.rpc_peer.send_rpc_notification("update",
            &json!({
                "view_id": view_id,
                "update": update,
            }));
    }

    pub fn scroll_to(&self, view_id: &str, line: usize, col: usize) {
        self.rpc_peer.send_rpc_notification("scroll_to",
            &json!({
                "view_id": view_id,
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
