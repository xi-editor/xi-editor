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

//! Plugins and related functionality.

use std::env;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak, mpsc};
use std::thread;
use std::collections::BTreeMap;
use std::process::{Command, Stdio, ChildStdin, Child};
use std::io::{BufReader, Write};

use serde_json::{self, Value};

use xi_rpc::{RpcLoop, RpcPeer, RpcCtx, Handler, Error};
use tabs::{BufferIdentifier, ViewIdentifier, BufferContainerRef};

pub type PluginPeer = RpcPeer<ChildStdin>;

/// A running plugin.
#[allow(dead_code)]
pub struct Plugin<W: Write> {
    peer: PluginPeer,
    /// The plugin's process
    process: Child,
    manager: Weak<Mutex<PluginManager<W>>>,
    description: PluginDescription,
    //TODO: temporary, eventually ids (view ids?) should be passed back and forth with RPCs
    buffer_id: BufferIdentifier,
}

/// A convenience wrapper for passing around a reference to a plugin.
///
/// Note: A plugin is always owned by and used through a `PluginRef`.
pub struct PluginRef<W: Write>(Arc<Mutex<Plugin<W>>>);

impl<W: Write> Clone for PluginRef<W> {
    fn clone(&self) -> Self {
        PluginRef(self.0.clone())
    }
}

impl<W: Write + Send + 'static> Handler<ChildStdin> for PluginRef<W> {
    fn handle_notification(&mut self, _ctx: RpcCtx<ChildStdin>, method: &str, params: &Value) {
        let _ = self.rpc_handler(method, params);
        // TODO: should check None
    }

    fn handle_request(&mut self, _ctx: RpcCtx<ChildStdin>, method: &str, params: &Value) ->
        Result<Value, Value> {
        let result = self.rpc_handler(method, params);
        result.ok_or_else(|| Value::String("missing return value".to_string()))
    }
}

impl<W: Write + Send + 'static> PluginRef<W> {
    fn rpc_handler(&self, method: &str, params: &Value) -> Option<Value> {
        let plugin_manager = {
            self.0.lock().unwrap().manager.upgrade()
        };

        if let Some(plugin_manager) = plugin_manager {
            let cmd = serde_json::from_value::<PluginCommand>(params.to_owned())
                .expect(&format!("failed to parse plugin rpc {}, params {:?}",
                        method, params));
            let result = plugin_manager.lock().unwrap()
                .handle_plugin_cmd(cmd, &self.0.lock().unwrap().buffer_id);
        result
        } else {
            None
        }
    }

    /// Init message sent to the plugin.
    pub fn init_buf(&self, buf_size: usize, rev: usize) {
        let plugin = self.0.lock().unwrap();
        let params = json!({
            "buf_size": buf_size,
            "rev": rev,
        });
        plugin.peer.send_rpc_notification("init_buf", &params);
    }

    /// Update message sent to the plugin.
    pub fn update<F>(&self, update: &PluginUpdate, callback: F)
            where F: FnOnce(Result<Value, Error>) + Send + 'static {
        let params = serde_json::to_value(update).expect("PluginUpdate invalid");
        self.0.lock().unwrap().peer.send_rpc_request_async("update", &params, callback);
    }
}


// example plugins. Eventually these should be loaded from disk.
pub fn debug_plugins() -> Vec<PluginDescription> {
    let mut path_base = env::current_exe().unwrap();
    path_base.pop();
    let make_path = |p: &str| -> PathBuf {
        let mut pb = path_base.clone();
        pb.push(p);
        pb
    };

    vec![
        PluginDescription::new("syntect", "0.0", make_path("xi-syntect-plugin")),
        PluginDescription::new("braces", "0.0", make_path("bracket_example.py")),
        PluginDescription::new("spellcheck", "0.0", make_path("spellcheck.py")),
        PluginDescription::new("shouty", "0.0", make_path("shouty.py")),
    ]
}

/// Describes attributes and capabilities of a plugin.
///
/// Note: - these will eventually be loaded from manifest files.
#[derive(Debug, Clone)]
pub struct PluginDescription {
    pub name: String,
    version: String,
    //scope: PluginScope,
    // more metadata ...
    /// path to plugin executable
    pub exec_path: PathBuf,
}

impl PluginDescription {
    fn new<S, P>(name: S, version: S, exec_path: P) -> Self
        where S: Into<String>, P: Into<PathBuf>
    {
        PluginDescription {
            name: name.into(),
            version: version.into(),
            exec_path: exec_path.into()
        }
    }

    /// Starts the executable described in this `PluginDescription`.
    fn launch<W, C>(&self, manager_ref: &Arc<Mutex<PluginManager<W>>>, buffer_id: &str, completion: C)
        where W: Write + Send + 'static,
              C: FnOnce(Result<PluginRef<W>, &'static str>) + Send + 'static
              // TODO: a real result type
    {
        let path = self.exec_path.clone();
        let buffer_id = buffer_id.to_owned();
        let manager_ref = manager_ref.clone();
        let description = self.clone();

        thread::spawn(move || {
            print_err!("starting plugin at path {:?}", path);
            let mut child = Command::new(&path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .expect("plugin failed to start");
            let child_stdin = child.stdin.take().unwrap();
            let child_stdout = child.stdout.take().unwrap();
            let mut looper = RpcLoop::new(child_stdin);
            let peer = looper.get_peer();
            peer.send_rpc_notification("ping", &Value::Array(Vec::new()));
            let plugin = Plugin {
                peer: peer,
                //TODO: I had the bright idea of keeping this reference but
                // I'm not sure exactly what to do with it (stopping the plugin is one thing)
                process: child,
                manager: Arc::downgrade(&manager_ref),
                description: description,
                buffer_id: buffer_id,
            };
            let mut plugin_ref = PluginRef(Arc::new(Mutex::new(plugin)));
            completion(Ok(plugin_ref.clone()));
            looper.mainloop(|| BufReader::new(child_stdout), &mut plugin_ref);
        });
    }
}


type PluginName = String;

/// Manages plugin loading, activation, lifecycle, and dispatch.
pub struct PluginManager<W: Write> {
    catalog: Vec<PluginDescription>,
    running: BTreeMap<(BufferIdentifier, PluginName), PluginRef<W>>,
    buffers: BufferContainerRef<W>,
}

impl<W: Write> PluginManager<W> {
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
}

impl<W: Write + Send + 'static> PluginManager<W> {

    //fn new_buffer(&mut self, view_id: &str) {}
    //fn file_open(&mut self, view_id: &str, path: &Path) {}
    //fn file_save(&mut self, view_id: &str, path: &Path) {}
    //fn view_close(&mut self, view_id: &str) {}

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
    fn handle_plugin_cmd(&self, cmd: PluginCommand, view_id: &ViewIdentifier) -> Option<Value> {
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


/// Starts a thread which collects editor updates and propagates them to plugins.
///
/// In addition to updates caused by user edits, updates can be caused by
/// plugin edits. These updates arrive asynchronously. After being applied to
/// the relevant buffer via an `Editor` instance, they need to be propagated
/// back out to all interested plugins.
///
/// In order to avoid additional complexity in the model graph (e.g. giving each
/// `Editor` a weak reference to the `PluginManager`) we instead give each
/// `Editor` a tx end of an `mpsc::channel`. As plugin updates are generated,
/// they are sent over this channel to a receiver running in another thread,
/// which forwards them to interested plugins.
pub fn start_update_thread<W: Write + Send + 'static>(
    rx: mpsc::Receiver<(ViewIdentifier, PluginUpdate, usize)>,
    plugins: Arc<Mutex<PluginManager<W>>>)
{
    thread::spawn(move ||{
        loop {
            let (view_id, update, undo_group) = rx.recv().unwrap();
            plugins.lock().unwrap().update(&view_id, update, undo_group);
        }
    });
    //FIXME: any reason not to drop the handle, here?
}

//TODO: Much of this might live somewhere else, and be shared with the plugin lib.

// ============================================================================
// Plugin RPC Types
// ============================================================================

/// An update event sent to a plugin.
#[derive(Serialize, Deserialize, Debug)]
pub struct PluginUpdate {
    start: usize,
    end: usize,
    new_len: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    rev: usize,
    edit_type: String,
    author: String,
}

impl PluginUpdate {
    pub fn new(start: usize, end: usize, new_len: usize, rev: usize,
               text: Option<String>, edit_type: String, author: String) -> Self {
        PluginUpdate {
            start: start,
            end: end,
            new_len: new_len,
            text: text,
            rev: rev,
            edit_type: edit_type,
            author: author
        }
    }
}

/// An simple edit, received from a plugin.
#[derive(Serialize, Deserialize, Debug)]
pub struct PluginEdit {
    pub start: u64,
    pub end: u64,
    pub rev: u64,
    pub text: String,
    /// the edit priority determines the resolution strategy when merging
    /// concurrent edits. The highest priority edit will be applied last.
    pub priority: u64,
    /// whether the inserted text prefers to be to the right of the cursor.
    pub after_cursor: bool,
    /// the originator of this edit: some identifier (plugin name, 'core', etc)
    pub author: String,
}

/// A response to an `update` RPC sent to a plugin.
#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum UpdateResponse {
    /// An edit to the buffer.
    Edit(PluginEdit),
    /// An acknowledgement with no action. A response cannot be Null, so we send a uint.
    Ack(u64),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub fg: u32,
    #[serde(rename = "font")]
    pub font_style: u8,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
/// RPC commands sent from the plugins.
pub enum PluginCommand {
    SetFgSpans {start: usize, len: usize, spans: Vec<Span>, rev: usize },
    GetData { offset: usize, max_size: usize, rev: usize },
    Alert { msg: String },
    LineCount,
}
