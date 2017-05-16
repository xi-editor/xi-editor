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
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::process::{ChildStdin, Child, Command, Stdio};
use std::io::{self, BufReader, Write};

use serde_json::{self, Value};

use xi_rpc::{self, RpcPeer, RpcCtx, RpcLoop, Handler};
use tabs::ViewIdentifier;

pub use self::manager::{PluginManagerRef, WeakPluginManagerRef};

use self::rpc_types::{PluginUpdate, PluginCommand, PluginBufferInfo};
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
    view_id: ViewIdentifier,
}

/// A convenience wrapper for passing around a reference to a plugin.
///
/// Note: A plugin is always owned by and used through a `PluginRef`.
///
/// The second field is used to flag dead plugins for cleanup.
pub struct PluginRef<W: Write>(Arc<Mutex<Plugin<W>>>, Arc<AtomicBool>);

impl<W: Write> Clone for PluginRef<W> {
    fn clone(&self) -> Self {
        PluginRef(self.0.clone(), self.1.clone())
    }
}

impl<W: Write + Send + 'static> Handler<ChildStdin> for PluginRef<W> {
    fn handle_notification(&mut self, _ctx: RpcCtx<ChildStdin>, method: &str, params: &Value) {
        if let Some(_) = self.rpc_handler(method, params) {
            print_err!("Unexpected return value for notification {}", method)
        }
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
                cmd, &self.0.lock().unwrap().view_id);
        result
        } else {
            None
        }
    }

    /// Initialize the plugin.
    pub fn initialize(&self, init: &PluginBufferInfo) {
        self.0.lock().unwrap().peer
            .send_rpc_notification("initialize", &json!({
                "buffer_info": init,
            }));
    }

    /// Update message sent to the plugin.
    pub fn update<F>(&self, update: &PluginUpdate, callback: F)
            where F: FnOnce(Result<Value, xi_rpc::Error>) + Send + 'static {
        let params = serde_json::to_value(update).expect("PluginUpdate invalid");
        match self.0.lock() {
            Ok(plugin) => plugin.peer.send_rpc_request_async("update", &params, callback),
            Err(err) => { 
                print_err!("plugin update failed {:?}", err);
                callback(Err(xi_rpc::Error::PeerDisconnect));
            }
        }
    }

    /// Termination message sent to the plugin.
    ///
    /// The plugin is expected to clean up and close the pipe.
    pub fn shutdown(&self) {
        match self.0.lock() {
            Ok(mut inner) => {
                //FIXME: don't block here?
                inner.peer.send_rpc_notification("shutdown", &json!({}));
                // TODO: get rust plugin lib to respect shutdown msg
                if inner.description.name == "syntect" {
                    let _ = inner.process.kill();
                }
                print_err!("waiting on process {}", inner.process.id());
                let exit_status = inner.process.wait();
                print_err!("process ended {:?}", exit_status);
            }
            Err(_) => print_err!("plugin mutex poisoned"),
        }
    }

    /// Returns `true` if this plugin has crashed.
    pub fn is_dead(&self) -> bool {
        self.1.load(Ordering::SeqCst)
    }

    /// Marks this plugin as having crashed.
    pub fn declare_dead(&mut self) {
        self.1.store(true, Ordering::SeqCst);
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
    manager_ref: &PluginManagerRef<W>)
{
    let manager_ref = manager_ref.clone();
    thread::spawn(move ||{
        loop {
            match rx.recv() {
                Ok((view_id, update, undo_group)) => {
                    manager_ref.update_plugins(&view_id, update, undo_group);
                }
                Err(_) => break,
            }
        }
    });
}

/// Launches a plugin, associating it with a given view.
pub fn start_plugin<W, C>(manager_ref: &PluginManagerRef<W>,
                          plugin_desc: &PluginDescription,
                          view_id: &ViewIdentifier,
                          completion: C)
    where W: Write + Send + 'static,
          C: FnOnce(Result<PluginRef<W>, io::Error>) + Send + 'static
{

    let manager_ref = manager_ref.to_weak();
    let view_id = view_id.to_owned();
    let plugin_desc = plugin_desc.to_owned();

    thread::spawn(move || {
        print_err!("starting plugin at path {:?}", &plugin_desc.exec_path);
        let child = Command::new(&plugin_desc.exec_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn();

        match child {
            Ok(mut child) => {
                let child_stdin = child.stdin.take().unwrap();
                let child_stdout = child.stdout.take().unwrap();
                let mut looper = RpcLoop::new(child_stdin);
                let peer = looper.get_peer();
                peer.send_rpc_notification("ping", &Value::Array(Vec::new()));
                let plugin = Plugin {
                    peer: peer,
                    process: child,
                    manager: manager_ref,
                    description: plugin_desc,
                    view_id: view_id,
                };
                let mut plugin_ref = PluginRef(
                    Arc::new(Mutex::new(plugin)),
                    Arc::new(AtomicBool::new(false)));
                completion(Ok(plugin_ref.clone()));
                looper.mainloop(|| BufReader::new(child_stdout), &mut plugin_ref);
            }
            Err(err) => completion(Err(err)),
        }
    });
}
