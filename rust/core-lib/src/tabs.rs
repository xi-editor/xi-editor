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
use rpc::{CoreCommand, EditCommand, PluginCommand};
use styles::{Style, StyleMap};
use MainPeer;
use plugins::PluginManager;

/// ViewIdentifiers are the primary means of routing messages between xi-core and a client view.
pub type ViewIdentifier = String;

/// BufferIdentifiers uniquely identify open buffers.
pub type BufferIdentifier = String;

// TODO: proposed new name: something like "Core" or "CoreState" or "EditorState"? "Documents?"
pub struct Tabs<W: Write> {
    /// maps file names to buffer identifiers. If a client asks to open a file that is already
    /// open, we treat it as a request for a new view.
    open_files: BTreeMap<PathBuf, BufferIdentifier>,
    // TODO: maybe some ActiveBuffersRef type, to replace buffers/views here?
    /// maps buffer identifiers (filenames) to editor instances
    buffers: Arc<Mutex<BTreeMap<BufferIdentifier, Editor<W>>>>,
    /// maps view identifiers to editor instances. All actions originate in a view; this lets us
    /// route messages correctly when multiple views share a buffer.
    views: BTreeMap<ViewIdentifier, BufferIdentifier>,
    id_counter: usize,
    kill_ring: Arc<Mutex<Rope>>,
    style_map: Arc<Mutex<StyleMap>>,
    plugins: Arc<Mutex<PluginManager<W>>>,
}

#[derive(Clone)]
pub struct TabCtx<W: Write> {
    kill_ring: Arc<Mutex<Rope>>,
    rpc_peer: MainPeer<W>,
    style_map: Arc<Mutex<StyleMap>>,
    plugins: Arc<Mutex<PluginManager<W>>>,
}


impl<W: Write + Send + 'static> Tabs<W> {
    pub fn new() -> Tabs<W> {
        let buffers = Arc::new(Mutex::new(BTreeMap::new()));
        Tabs {
            open_files: BTreeMap::new(),
            buffers: buffers.clone(),
            views: BTreeMap::new(),
            id_counter: 0,
            kill_ring: Arc::new(Mutex::new(Rope::from(""))),
            style_map: Arc::new(Mutex::new(StyleMap::new())),
            plugins: Arc::new(Mutex::new(PluginManager::new(buffers))),
        }
    }

    fn new_tab_ctx(&self, peer: &MainPeer<W>) -> TabCtx<W> {
        TabCtx {
            kill_ring: self.kill_ring.clone(),
            rpc_peer: peer.clone(),
            style_map: self.style_map.clone(),
            plugins: self.plugins.clone(),
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

            NewView { file_path } => Some(self.do_new_view(rpc_peer, file_path)),
            Save { view_id, file_path } => self.do_save(view_id, file_path),
            Edit { view_id, edit_command } => self.do_edit(view_id, edit_command),
            Plugin { plugin_command } => self.do_plugin_cmd(plugin_command),
        }
    }

    /// Creates a new view and associates it with a buffer.
    ///
    /// This function always creates a new view and associates it with a buffer
    /// (which we access through an `Editor` instance). This buffer may be existing,
    /// or it may be created.
    ///
    /// A `new_view` request is handled differently depending on the `file_path`
    /// argument, and on application state. If `file_path` is given and a buffer
    /// associated with that file is already open, we create a new view into the
    /// existing buffer. If `file_path` is given and that file _isn't_ open,
    /// we load that file into a new buffer. If `file_path` is not given,
    /// we create a new empty buffer.
    fn do_new_view(&mut self, rpc_peer: &MainPeer<W>, file_path: Option<&str>) -> Value {
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
                let mut editor_map = self.buffers.lock().unwrap();
                editor_map.get_mut(&buffer_id).unwrap().set_path(&file_path);
            }
        } else {
            // file_path was nil: create a new empty buffer.
            let buffer_id = self.next_buffer_id();
            self.new_empty_view(rpc_peer, &view_id, buffer_id);
        }
        json!({
            "view_id": view_id,
            //TODO: this should be determined based on filetype etc
            "available_plugins": self.plugins.lock().unwrap().debug_available_plugins(),
        })
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
        let mut editor_map = self.buffers.lock().unwrap();
        let editor = editor_map.get_mut(&buffer_id).expect("missing editor_id for view_id");
        self.views.insert(view_id.to_owned(), buffer_id);
        editor.add_view(view_id);
    }

    fn finalize_new_view(&mut self, view_id: &str, buffer_id: String, editor: Editor<W>) {
        self.views.insert(view_id.to_owned(), buffer_id.clone());
        self.buffers.lock().unwrap().insert(buffer_id, editor);
    }

    fn read_file<P: AsRef<Path>>(&self, path: P) -> io::Result<String> {
        let mut f = File::open(path)?;
        let mut s = String::new();
        f.read_to_string(&mut s)?;
        Ok(s)
    }

    fn close_view(&mut self, view_id: &str) {
        let buffer_id = self.views.remove(view_id).expect("missing buffer id when closing view");
        let (has_views, path) = {
            let mut editor_map = self.buffers.lock().unwrap();
            let editor = editor_map.get_mut(&buffer_id).expect("missing editor when closing view");
            editor.remove_view(view_id);
            (editor.has_views(), editor.get_path().map(PathBuf::from))
        };

        if !has_views {
            self.buffers.lock().unwrap().remove(&buffer_id);
            if let Some(path) = path {
                self.open_files.remove(&path);
            }
        }
    }

    fn do_save(&mut self, view_id: &str, file_path: &str) -> Option<Value> {
        let buffer_id = self.views.get(view_id)
            .expect(&format!("missing buffer id for view {}", view_id));
        let mut editor_map = self.buffers.lock().unwrap();
        let editor = editor_map.get_mut(buffer_id)
            .expect(&format!("missing editor for buffer {}", buffer_id));
        let file_path = PathBuf::from(file_path);

        // if this is a new path for an existing file, we have a bit of housekeeping to do:
        if let Some(prev_path) = editor.get_path() {
            if prev_path != file_path {
                self.open_files.remove(prev_path);
            }
        }
        editor.do_save(&file_path);
        self.open_files.insert(file_path, buffer_id.to_owned());
        None
    }

    fn do_edit(&mut self, view_id: &str, cmd: EditCommand) -> Option<Value> {
        if let Some(buffer_id) = self.views.get(view_id) {
            let (should_update, result, undo_group) = {
                let mut editor_map = self.buffers.lock().unwrap();
                let editor = editor_map.get_mut(buffer_id).unwrap();
                let result = editor.do_rpc_impl(view_id, cmd);
                let should_update = editor.get_last_plugin_update();
                let undo_group = editor.get_last_undo_group();
                (should_update, result, undo_group)
            };
            if should_update.is_some() {
                self.plugins.lock().unwrap()
                    .update(buffer_id, should_update.unwrap(), undo_group);
            }
            result

        } else {
            // should this just be a crash?
            print_err!("missing buffer_id for view {}", view_id);
            None
        }
    }

    fn do_plugin_cmd(&mut self, cmd: PluginCommand) -> Option<Value> {
        use self::PluginCommand::*;
        match cmd {
            Start { view_id, plugin_name } => {
                let buffer_id = self.views.get(&view_id)
                    .expect(&format!("missing buffer id for view {}", view_id));
                // TODO: this is a hack, there are different ways a plugin might be launched
                // and they would have different init params, this is just mimicing old api
                let (buf_size, _, rev) = {
                    let editor_map = self.buffers.lock().unwrap();
                    let editor = editor_map.get(buffer_id).unwrap();
                    let params = editor.plugin_init_params();
                    params
                };

                self.plugins.lock().unwrap().start_plugin(
                    &self.plugins, buffer_id, &plugin_name, buf_size, rev);
            }
            //TODO: stop a plugin
            //Stop { view_id, plugin_name } => (),
            _ => (),
        }
        None
    }

    pub fn handle_idle(&self) {
        for editor in self.buffers.lock().unwrap().values_mut() {
            editor.render();
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

    pub fn available_plugins(&self, view_id: &str, plugins: Vec<&str>) {
        self.rpc_peer.send_rpc_notification("available_plugins",
                                            &json!({
                                                "view_id": view_id,
                                                "plugins": plugins }));
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
