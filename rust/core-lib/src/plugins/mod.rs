// Copyright 2017 The xi-editor Authors.
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

mod catalog;
pub mod manifest;
pub mod rpc;

use std::fmt;
use std::io::BufReader;
use std::path::Path;
use std::process::{Child, Command as ProcCommand, Stdio};
use std::sync::Arc;
use std::thread;

use serde_json::Value;

use xi_rpc::{self, RpcLoop, RpcPeer};

use crate::config::Table;
use crate::syntax::LanguageId;
use crate::tabs::ViewId;
use crate::WeakXiCore;

use self::rpc::{PluginBufferInfo, PluginUpdate};

pub(crate) use self::catalog::PluginCatalog;
pub use self::manifest::{Command, PlaceholderRpc, PluginDescription};

pub type PluginName = String;

/// A process-unique identifier for a running plugin.
///
/// Note: two instances of the same executable will have different identifiers.
/// Note: this identifier is distinct from the OS's process id.
#[derive(
    Serialize, Deserialize, Default, Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord,
)]
pub struct PluginPid(pub(crate) usize);

pub type PluginId = PluginPid;

impl fmt::Display for PluginPid {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "plugin-{}", self.0)
    }
}

pub struct Plugin {
    peer: RpcPeer,
    pub(crate) id: PluginId,
    pub(crate) name: String,
    #[allow(dead_code)]
    process: Child,
}

impl Plugin {
    //TODO: initialize should be sent automatically during launch,
    //and should only send the plugin_id. We can just use the existing 'new_buffer'
    // RPC for adding views
    pub fn initialize(&self, info: Vec<PluginBufferInfo>) {
        self.peer.send_rpc_notification(
            "initialize",
            &json!({
                "plugin_id": self.id,
                "buffer_info": info,
            }),
        )
    }

    pub fn shutdown(&self) {
        self.peer.send_rpc_notification("shutdown", &json!({}));
    }

    // TODO: rethink naming, does this need to be a vec?
    pub fn new_buffer(&self, info: &PluginBufferInfo) {
        self.peer.send_rpc_notification("new_buffer", &json!({ "buffer_info": [info] }))
    }

    pub fn close_view(&self, view_id: ViewId) {
        self.peer.send_rpc_notification("did_close", &json!({ "view_id": view_id }))
    }

    pub fn did_save(&self, view_id: ViewId, path: &Path) {
        self.peer.send_rpc_notification(
            "did_save",
            &json!({
                "view_id": view_id,
                "path": path,
            }),
        )
    }

    pub fn update<F>(&self, update: &PluginUpdate, callback: F)
    where
        F: FnOnce(Result<Value, xi_rpc::Error>) + Send + 'static,
    {
        self.peer.send_rpc_request_async("update", &json!(update), Box::new(callback))
    }

    pub fn toggle_tracing(&self, enabled: bool) {
        self.peer.send_rpc_notification("tracing_config", &json!({ "enabled": enabled }))
    }

    pub fn collect_trace(&self) -> Result<Value, xi_rpc::Error> {
        self.peer.send_rpc_request("collect_trace", &json!({}))
    }

    pub fn config_changed(&self, view_id: ViewId, changes: &Table) {
        self.peer.send_rpc_notification(
            "config_changed",
            &json!({
                "view_id": view_id,
                "changes": changes,
            }),
        )
    }

    pub fn language_changed(&self, view_id: ViewId, new_lang: &LanguageId) {
        self.peer.send_rpc_notification(
            "language_changed",
            &json!({
                "view_id": view_id,
                "new_lang": new_lang,
            }),
        )
    }

    pub fn get_hover(&self, view_id: ViewId, request_id: usize, position: usize) {
        self.peer.send_rpc_notification(
            "get_hover",
            &json!({
                "view_id": view_id,
                "request_id": request_id,
                "position": position,
            }),
        )
    }

    pub fn dispatch_command(&self, view_id: ViewId, method: &str, params: &Value) {
        self.peer.send_rpc_notification(
            "custom_command",
            &json!({
                "view_id": view_id,
                "method": method,
                "params": params,
            }),
        )
    }
}

pub(crate) fn start_plugin_process(
    plugin_desc: Arc<PluginDescription>,
    id: PluginId,
    core: WeakXiCore,
) {
    let spawn_result = thread::Builder::new()
        .name(format!("<{}> core host thread", &plugin_desc.name))
        .spawn(move || {
            info!("starting plugin {}", &plugin_desc.name);
            let child = ProcCommand::new(&plugin_desc.exec_path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn();

            match child {
                Ok(mut child) => {
                    let child_stdin = child.stdin.take().unwrap();
                    let child_stdout = child.stdout.take().unwrap();
                    let mut looper = RpcLoop::new(child_stdin);
                    let peer: RpcPeer = Box::new(looper.get_raw_peer());
                    let name = plugin_desc.name.clone();
                    peer.send_rpc_notification("ping", &Value::Array(Vec::new()));
                    let plugin = Plugin { peer, process: child, name, id };

                    // set tracing immediately
                    if xi_trace::is_enabled() {
                        plugin.toggle_tracing(true);
                    }

                    core.plugin_connect(Ok(plugin));
                    let mut core = core;
                    let err = looper.mainloop(|| BufReader::new(child_stdout), &mut core);
                    core.plugin_exit(id, err);
                }
                Err(err) => core.plugin_connect(Err(err)),
            }
        });

    if let Err(err) = spawn_result {
        error!("thread spawn failed for {}, {:?}", id, err);
    }
}
