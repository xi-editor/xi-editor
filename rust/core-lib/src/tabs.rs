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

//! A container for all the documents being edited. Also functions as main dispatch for RPC.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::path::{PathBuf, Path};
use std::fs::File;
use std::sync::{Arc, Mutex, MutexGuard, Weak, mpsc};

use serde_json::value::Value;

use xi_rope::rope::Rope;
use editor::Editor;
use rpc::{CoreCommand, EditCommand, PluginCommand};
use styles::{Style, StyleMap};
use MainPeer;
use syntax::SyntaxDefinition;
use plugins::{self, PluginManagerRef};
use plugins::rpc_types::PluginUpdate;

/// ViewIdentifiers are the primary means of routing messages between xi-core and a client view.
pub type ViewIdentifier = String;

/// BufferIdentifiers uniquely identify open buffers.
pub type BufferIdentifier = String;


/// Tracks open buffers, and relationships between buffers and views.
pub struct BufferContainer<W: Write> {
    /// associates open file paths to buffers
    open_files: BTreeMap<PathBuf, BufferIdentifier>,
    /// maps buffer identifiers (filenames) to editor instances
    editors: BTreeMap<BufferIdentifier, Editor<W>>,
    /// maps view identifiers to buffer identifiers. All actions originate in a view;
    /// this lets us route messages correctly when multiple views share a buffer.
    views: BTreeMap<ViewIdentifier, BufferIdentifier>,
}

/// Wrapper around `Arc<Mutex<`[`BufferContainer`][BufferContainer]`>>`,
/// for more ergonomic synchronization.
///
/// `BufferContainerRef` provides a thread-safe API for accessing and modifying the
/// [`BufferContainer`][BufferContainer]. All methods on `BufferContainerRef`
/// are thread safe. For finer grained actions, the underlying container can be accessed
/// via `BufferContainer::lock`.
///
/// [BufferContainer]: struct.BufferContainer.html
pub struct BufferContainerRef<W: Write>(Arc<Mutex<BufferContainer<W>>>);

/// Wrapper around a `Weak<Mutex<BufferContainer<W>>>`
///
/// `WeakBufferContaienrRef` provides a more ergonomic way of storing a `Weak`
/// reference to a [`BufferContainer`][BufferContainer].
///
/// [BufferContainer]: struct.BufferContainer.html
pub struct WeakBufferContainerRef<W: Write>(Weak<Mutex<BufferContainer<W>>>);

impl <W:Write>BufferContainer<W> {
    /// Returns a reference to the `Editor` instance owning `view_id`'s view.
    ///
    /// Panics if no buffer is associated with `view_id`.
    pub fn editor_for_view(&self, view_id: &ViewIdentifier) -> Option<&Editor<W>> {
        let buffer_id = self.views.get(view_id)
            .expect(&format!("no buffer_id for view {}", view_id));
        self.editors.get(buffer_id)
    }

    /// Returns a mutable reference to the `Editor` instance owning `view_id`'s view.
    ///
    /// Panics if no buffer is associated with `view_id`.
    pub fn editor_for_view_mut(&mut self, view_id: &ViewIdentifier) -> Option<&mut Editor<W>> {
        let buffer_id = self.views.get(view_id)
            .expect(&format!("no buffer_id for view {}", view_id));
        self.editors.get_mut(buffer_id)
    }
}

impl <W: Write + Send + 'static>BufferContainerRef<W> {
    pub fn new() -> Self {
        BufferContainerRef(Arc::new(Mutex::new(
                    BufferContainer {
                        open_files: BTreeMap::new(),
                        editors: BTreeMap::new(),
                        views: BTreeMap::new(),
                    })))
    }

    /// Returns a handle to the inner `MutexGuard`.
    pub fn lock(&self) -> MutexGuard<BufferContainer<W>> {
        self.0.lock().unwrap()
    }

    /// Creates a new `WeakBufferContainerRef<W>`.
    pub fn to_weak(&self) -> WeakBufferContainerRef<W> {
        let weak_inner = Arc::downgrade(&self.0);
        WeakBufferContainerRef(weak_inner)
    }

    /// Returns `true` if `file_path` is already open, else `false`.
    pub fn has_open_file<P: AsRef<Path>>(&self, file_path: P) -> bool {
        self.lock().open_files.contains_key(file_path.as_ref())
    }

    /// Adds a new editor, associating it with the provided identifiers.
    pub fn add_editor(&self, view_id: &ViewIdentifier, buffer_id: &BufferIdentifier,
                      editor: Editor<W>) {
        let mut inner = self.lock();
        inner.views.insert(view_id.to_owned(), buffer_id.to_owned());
        inner.editors.insert(buffer_id.to_owned(), editor);
    }

    /// Registers `file_path` as an open file, associated with `view_id`'s buffer.
    ///
    /// If an existing path is already associated with this buffer, it is removed.
    pub fn set_path<P: AsRef<Path>>(&self, file_path: P, view_id: &ViewIdentifier) {
        let file_path = file_path.as_ref();
        let mut inner = self.lock();
        let buffer_id = inner.views.get(view_id).unwrap().to_owned();
        let prev_path = inner.editor_for_view(view_id).unwrap()
            .get_path().map(Path::to_owned);
        if let Some(prev_path) = prev_path {
            if prev_path != file_path {
                inner.open_files.remove(&prev_path);
            }
        }
        inner.open_files.insert(file_path.to_owned(), buffer_id);
        inner.editor_for_view_mut(view_id).unwrap()._set_path(file_path);
    }

    /// Adds a new view to the `Editor` instance owning `buffer_id`.
    pub fn add_view(&self, view_id: &ViewIdentifier, buffer_id: &BufferIdentifier) {
        let mut inner = self.lock();
        inner.views.insert(view_id.to_owned(), buffer_id.to_owned());
        inner.editor_for_view_mut(view_id).unwrap().add_view(view_id);
    }

    /// Closes the view with identifier `view_id`.
    ///
    /// If this is the last view open onto the underlying buffer, also cleans up
    /// the `Editor` instance.
    pub fn close_view(&self, view_id: &ViewIdentifier) {
        let path_to_remove = {
            let mut inner = self.lock();
            let editor = inner.editor_for_view_mut(view_id).unwrap();
            editor.remove_view(view_id);
            if !editor.has_views() {
                editor.get_path().map(PathBuf::from)
            } else {
                None
            }
        };

        if let Some(path) = path_to_remove {
            let mut inner = self.lock();
            let buffer_id = inner.views.remove(view_id).unwrap();
            inner.open_files.remove(&path);
            inner.editors.remove(&buffer_id);
        }
    }
}

impl <W: Write>WeakBufferContainerRef<W> {
    /// Upgrades the weak reference to an Arc, if possible.
    ///
    /// Returns `None` if the inner value has been deallocated.
    pub fn upgrade(&self) -> Option<BufferContainerRef<W>> {
        match self.0.upgrade() {
            Some(inner) => Some(BufferContainerRef(inner)),
            None => None
        }
    }
}

impl<W: Write> Clone for BufferContainerRef<W> {
    fn clone(&self) -> Self {
        BufferContainerRef(self.0.clone())
    }
}

/// A container for all open documents.
///
/// `Documents` is effectively the apex of the xi's model graph. It keeps references
/// to all active `Editor ` instances (through a `BufferContainerRef` instance),
/// and handles dispatch of RPC methods between client views and `Editor`
/// instances, as well as between `Editor` instances and Plugins.
pub struct Documents<W: Write> {
    /// keeps track of buffer/view state.
    buffers: BufferContainerRef<W>,
    id_counter: usize,
    kill_ring: Arc<Mutex<Rope>>,
    style_map: Arc<Mutex<StyleMap>>,
    plugins: PluginManagerRef<W>,
    /// A tx channel used to propagate plugin updates from all `Editor`s.
    update_channel: mpsc::Sender<(ViewIdentifier, PluginUpdate, usize)>
}

#[derive(Clone)]
/// A container for state shared between `Editor` instances.
pub struct DocumentCtx<W: Write> {
    kill_ring: Arc<Mutex<Rope>>,
    rpc_peer: MainPeer<W>,
    style_map: Arc<Mutex<StyleMap>>,
    update_channel: mpsc::Sender<(ViewIdentifier, PluginUpdate, usize)>
}


impl<W: Write + Send + 'static> Documents<W> {
    pub fn new() -> Documents<W> {
        let buffers = BufferContainerRef::new();
        let plugin_manager = PluginManagerRef::new(buffers.clone());
        let (update_tx, update_rx) = mpsc::channel();

        plugins::start_update_thread(update_rx, &plugin_manager);

        Documents {
            buffers: buffers,
            id_counter: 0,
            kill_ring: Arc::new(Mutex::new(Rope::from(""))),
            style_map: Arc::new(Mutex::new(StyleMap::new())),
            plugins: plugin_manager,
            update_channel: update_tx,
        }
    }

    fn new_tab_ctx(&self, peer: &MainPeer<W>) -> DocumentCtx<W> {
        DocumentCtx {
            kill_ring: self.kill_ring.clone(),
            rpc_peer: peer.clone(),
            style_map: self.style_map.clone(),
            update_channel: self.update_channel.clone(),
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
                self.do_close_view(&view_id.to_owned());
                None
            },

            NewView { file_path } => Some(self.do_new_view(rpc_peer, file_path)),
            Save { view_id, file_path } => self.do_save(&view_id.to_owned(), file_path),
            Edit { view_id, edit_command } => self.do_edit(&view_id.to_owned(), edit_command),
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
            if self.buffers.has_open_file(&file_path) {
                let buffer_id = self.next_buffer_id();
                self.new_empty_view(rpc_peer, &view_id, buffer_id);
                // let buffer_id = self.open_files.get(&file_path).unwrap().to_owned();
                //self.add_view(&view_id, buffer_id);
            } else {
                // not open: create new buffer_id and open file
                let buffer_id = self.next_buffer_id();
                self.new_view_with_file(rpc_peer, &view_id, buffer_id.clone(), &file_path);
            }
        } else {
            // file_path was nil: create a new empty buffer.
            let buffer_id = self.next_buffer_id();
            self.new_empty_view(rpc_peer, &view_id, buffer_id);
        }
        json!(view_id)
    }

    fn do_close_view(&mut self, view_id: &ViewIdentifier) {
        self.buffers.close_view(view_id);
        self.plugins.document_close(view_id);
    }

    fn new_empty_view(&mut self, rpc_peer: &MainPeer<W>, view_id: &ViewIdentifier,
                      buffer_id: BufferIdentifier) {
        let editor = Editor::new(self.new_tab_ctx(rpc_peer), view_id);
        self.add_editor(view_id, &buffer_id, editor, None);
        self.plugins.document_new(view_id);
    }

    fn new_view_with_file(&mut self, rpc_peer: &MainPeer<W>, view_id: &ViewIdentifier,
                          buffer_id: BufferIdentifier, path: &Path) {
        match self.read_file(&path) {
            Ok(contents) => {
                let ed = Editor::with_text(self.new_tab_ctx(rpc_peer), view_id, contents);
                self.add_editor(view_id, &buffer_id, ed, Some(path));
                self.plugins.document_open(view_id);
            }
            Err(err) => {
                let ed = Editor::new(self.new_tab_ctx(rpc_peer), view_id);
                if path.exists() {
                    // if this is a read error of an actual file, we don't set path
                    // TODO: we should be reporting errors to the client
                    print_err!("unable to read file: {}, error: {:?}", buffer_id, err);
                    self.add_editor(view_id, &buffer_id, ed, None);
                } else {
                    // if a path that doesn't exist, create a new empty buffer + set path
                    self.add_editor(view_id, &buffer_id, ed, Some(path));
                }
                self.plugins.document_new(view_id);
            }
        }
    }

    /// Adds a new editor, associating it with the provided identifiers.
    ///
    /// This is called once each time a new editor is created.
    fn add_editor(&self, view_id: &ViewIdentifier, buffer_id: &BufferIdentifier,
                  editor: Editor<W>, path: Option<&Path>) {
        self.buffers.add_editor(view_id, buffer_id, editor);
        if let Some(path) = path {
            self.buffers.set_path(path, view_id);
        }
    }

    /// Adds a new view to an existing editor instance.
    #[allow(unreachable_code, unused_variables, dead_code)] 
    fn add_view(&mut self, view_id: &ViewIdentifier, buffer_id: BufferIdentifier) {
        panic!("add_view should not currently be accessible");
        self.buffers.add_view(view_id, &buffer_id);
    }

    fn read_file<P: AsRef<Path>>(&self, path: P) -> io::Result<String> {
        let mut f = File::open(path)?;
        let mut s = String::new();
        f.read_to_string(&mut s)?;
        Ok(s)
    }

    fn do_save<P: AsRef<Path>>(&mut self, view_id: &ViewIdentifier,
                               file_path: P) -> Option<Value> {
        //TODO: handle & report errors
        let file_path = file_path.as_ref();
        let prev_syntax = self.buffers.lock().editor_for_view(view_id)
            .unwrap().get_syntax().to_owned();
        // notify of syntax change before notify of file_save
        //FIXME: this doesn't tell us if the syntax _will_ change, for instance if syntax was a user
        //selection. (we don't handle this case right now)
        if prev_syntax != SyntaxDefinition::new(file_path.to_str()) {
                self.plugins.document_syntax_changed(view_id);
        }

        self.buffers.lock().editor_for_view_mut(view_id)
            .unwrap().do_save(file_path);
        self.buffers.set_path(file_path, view_id);
        self.plugins.document_did_save(&view_id);
        None
    }

    fn do_edit(&mut self, view_id: &ViewIdentifier, cmd: EditCommand) -> Option<Value> {
        self.buffers.lock().editor_for_view_mut(view_id).unwrap().do_rpc(view_id, cmd)
    }

    #[allow(unused_variables)]
    fn do_plugin_cmd(&mut self, cmd: PluginCommand) -> Option<Value> {
        use self::PluginCommand::*;
        match cmd {
            InitialPlugins { view_id } => Some(json!(
                    self.plugins.lock().debug_available_plugins())),
            Start { view_id, plugin_name } => {
                // TODO: this is a hack, there are different ways a plugin might be launched
                // and they would have different init params, this is just mimicing old api
                let (buf_size, _, rev) = {
                self.buffers.lock().editor_for_view(&view_id).unwrap()
                    .plugin_init_params()
                };

                //TODO: stop passing buffer ids
                self.plugins.start_plugin(&view_id, &plugin_name, buf_size, rev);
                None
            }
            //TODO: stop a plugin
            Stop { view_id, plugin_name } => None,
        }
    }

    pub fn handle_idle(&self) {
        for editor in self.buffers.lock().editors.values_mut() {
            editor.render();
        }
    }
}

impl<W: Write> DocumentCtx<W> {
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

    /// Notify plugins of an update
    pub fn update_plugins(&self, view_id: ViewIdentifier,
                          update: PluginUpdate, undo_group: usize) {
        self.update_channel.send((view_id, update, undo_group)).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xi_rpc::{RpcLoop};
    use std::env;
    use std::fs::File;

    // a bit of gymnastics to let us instantiate an Editor instance
    fn mock_doc_ctx(tempfile: &str) -> DocumentCtx<File> {
        let mut dir = env::temp_dir();
        dir.push(tempfile);
        let f = File::create(dir).unwrap();

        let mock_loop = RpcLoop::new(f);
        let mock_peer = mock_loop.get_peer();
        let (update_tx, _) = mpsc::channel();

        DocumentCtx {
            kill_ring: Arc::new(Mutex::new(Rope::from(""))),
            rpc_peer: mock_peer.clone(),
            style_map: Arc::new(Mutex::new(StyleMap::new())),
            update_channel: update_tx,
        }
    }

    #[test]
    fn test_save_as() {
        let container_ref = BufferContainerRef::new();
        assert!(!container_ref.has_open_file("a fake file, for sure"));
        let view_id_1 = "view-id-1".to_owned();
        let buf_id_1 = "buf-id-1".to_owned();
        let path_1 = PathBuf::from("a_path");
        let path_2 = PathBuf::from("a_different_path");
        let editor = Editor::new(mock_doc_ctx(&view_id_1), &view_id_1);
        container_ref.add_editor(&view_id_1, &buf_id_1, editor);
        assert_eq!(container_ref.lock().editors.len(), 1);

        // set path (as if on save)
        container_ref.set_path(&path_1, &view_id_1);
        assert_eq!(container_ref.has_open_file(&path_1), true);
        assert_eq!(
            container_ref.lock().editor_for_view(&view_id_1).unwrap().get_path(),
            Some(path_1.as_ref()));

        // then save somewhere else:
        container_ref.set_path(&path_2, &view_id_1);
        assert_eq!(container_ref.lock().editors.len(), 1);
        assert_eq!(container_ref.has_open_file(&path_1), false);
        assert_eq!(container_ref.has_open_file(&path_2), true);
        assert_eq!(
            container_ref.lock().editor_for_view(&view_id_1).unwrap().get_path(),
            Some(path_2.as_ref()));

        // reopen the original file:
        let view_id_2 = "view-id-2".to_owned();
        let buf_id_2 = "buf-id-2".to_owned();
        let editor = Editor::new(mock_doc_ctx(&view_id_2), &view_id_2);
        container_ref.add_editor(&view_id_2, &buf_id_2, editor);
        container_ref.set_path(&path_1, &view_id_2);
        assert_eq!(container_ref.lock().editors.len(), 2);
        assert_eq!(container_ref.has_open_file(&path_1), true);
        assert_eq!(container_ref.has_open_file(&path_2), true);

        container_ref.close_view(&view_id_1);
        assert_eq!(container_ref.lock().editors.len(), 1);
        assert_eq!(container_ref.has_open_file(&path_2), false);
        assert_eq!(container_ref.has_open_file(&path_1), true);

        container_ref.close_view(&view_id_2);
        assert_eq!(container_ref.has_open_file(&path_2), false);
        assert_eq!(container_ref.lock().editors.len(), 0);
    }
}
