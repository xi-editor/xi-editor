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

pub mod rpc;
//mod manager;
mod manifest;
mod catalog;

use std::fmt;
use std::io::BufReader;
use std::path::Path;
use std::process::{Child, Command as ProcCommand, Stdio};
use std::thread;

use serde_json::Value;

use xi_rpc::{RpcPeer, RpcLoop, Error as RpcError};
use xi_trace;

use WeakXiCore;
use tabs::ViewId;

use self::rpc::{PluginUpdate, PluginBufferInfo};

pub use self::manifest::{PluginDescription, Command, PlaceholderRpc};
pub(crate) use self::catalog::PluginCatalog;

pub type PluginName = String;

/// A process-unique identifier for a running plugin.
///
/// Note: two instances of the same executable will have different identifiers.
/// Note: this identifier is distinct from the OS's process id.
#[derive(Serialize, Deserialize, Default, Debug, Clone, Copy, Hash,
         PartialEq, Eq, PartialOrd, Ord)]
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
    process: Child,
}

impl Plugin {
    //TODO: initialize should be sent automatically during launch,
    //and should only send the plugin_id. We can just use the existing 'new_buffer'
    // RPC for adding views
    pub fn initialize(&self, info: Vec<PluginBufferInfo>) {
        self.peer.send_rpc_notification("initialize",
                                        &json!({
                                         "plugin_id": self.id,
                                         "buffer_info": info,
                                        }))
    }

    // TODO: rethink naming, does this need to be a vec?
    pub fn new_buffer(&self, info: &PluginBufferInfo) {
        self.peer.send_rpc_notification("new_buffer",
                                        &json!({
                                            "buffer_info": [info],
                                        }))
    }

    pub fn close_view(&self, view_id: ViewId) {
        self.peer.send_rpc_notification("did_close",
                                        &json!({
                                            "view_id": view_id,
                                        }))

    }

    pub fn did_save(&self, view_id: ViewId, path: &Path) {
        self.peer.send_rpc_notification("did_save",
                                        &json!({
                                            "view_id": view_id,
                                            "path": path,
                                        }))
    }

    pub fn update<F>(&self, update: &PluginUpdate, callback: F)
        where F: FnOnce(Result<Value, RpcError>) + Send + 'static
    {
        self.peer.send_rpc_request_async("update", &json!(update),
                                         Box::new(callback))
    }


    pub fn toggle_tracing(&self, enabled: bool) {
        self.peer.send_rpc_notification("tracing_config",
                                        &json!({"enabled": enabled}))
    }

    pub fn collect_trace(&self) -> Result<Value, RpcError> {
        self.peer.send_rpc_request("collect_trace", &json!({}))
    }
}

pub(crate) fn start_plugin_process(plugin_desc: PluginDescription,
                                    id: PluginId, core: WeakXiCore) {
    thread::spawn(move || {
        eprintln!("starting plugin {}", &plugin_desc.name);
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
                eprintln!("spawned {}", &name);
                peer.send_rpc_notification("ping", &json!({}));
                let plugin = Plugin { peer, process: child, name, id };

                // set tracing immediately
                if xi_trace::is_enabled() {
                    plugin.toggle_tracing(true);
                }

                core.plugin_connect(Ok(plugin));
                //TODO: we could be logging plugin exit results
                let mut core = core;
                let _ = looper.mainloop(|| BufReader::new(child_stdout),
                                        &mut core);
            }
            Err(err) => core.plugin_connect(Err(err)),
        }
    });
}
