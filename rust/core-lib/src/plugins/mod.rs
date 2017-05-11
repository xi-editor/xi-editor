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

pub mod rpc_types;
mod manager;
mod manifest;

use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::process::{ChildStdin, Child};
use std::io::Write;

use serde_json::{self, Value};

use xi_rpc::{RpcPeer, RpcCtx, Handler, Error};
use tabs::{BufferIdentifier, ViewIdentifier};

pub use self::manager::{PluginManagerRef, WeakPluginManagerRef};

use self::rpc_types::{PluginUpdate, PluginCommand};
use self::manifest::PluginDescription;


pub type PluginPeer = RpcPeer<ChildStdin>;

/// A running plugin.
#[allow(dead_code)]
pub struct Plugin<W: Write> {
    peer: PluginPeer,
    /// The plugin's process
    process: Child,
    manager: WeakPluginManagerRef<W>,
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
            let result = plugin_manager.lock().handle_plugin_cmd(
                cmd, &self.0.lock().unwrap().buffer_id);
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

    /// Termination message sent to the plugin.
    ///
    /// The plugin is expected to clean up and close the pipe.
    pub fn shutdown(&self) {
        match self.0.lock() {
            Ok(inner) => inner.peer.send_rpc_notification("shutdown", &json!({})),
            Err(_) => print_err!("plugin mutex poisoned"),
        }
        self.0.lock().unwrap().peer.send_rpc_notification("shutdown", &json!({}));
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
    plugins_ref: &PluginManagerRef<W>)
{
    let plugins_ref = plugins_ref.clone();
    thread::spawn(move ||{
        loop {
            match rx.recv() {
                Ok((view_id, update, undo_group)) => {
                    plugins_ref.lock().update(&view_id, update, undo_group);
                }
                Err(_) => break,
            }
        }
    });
}
