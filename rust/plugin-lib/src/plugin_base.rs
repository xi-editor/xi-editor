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

//! A base for xi plugins. Will be split out into its own crate once it's a bit more stable.

use std::io;
use std::path::{PathBuf, Path};

use serde_json::{self, Value};
use serde::Deserialize;

use xi_core::{ViewIdentifier, PluginPid, plugin_rpc, SyntaxDefinition,
ConfigTable, BufferConfig};
use xi_rpc::{self, RpcLoop, RpcPeer, RpcCtx, RemoteError, ReadError};

#[derive(Debug)]
pub enum Error {
    RpcError(xi_rpc::Error),
    WrongReturnType,
}

pub trait Handler {
    fn handle_notification(&mut self, ctx: PluginCtx,
                           rpc: plugin_rpc::HostNotification);
    fn handle_request(&mut self, ctx: PluginCtx, rpc: plugin_rpc::HostRequest)
                      -> Result<Value, RemoteError>;
    #[allow(unused_variables)]
    fn idle(&mut self, ctx: PluginCtx, token: usize) {}
}

/// A container for general view information, shared between all plugin layers.
pub struct ViewState {
    pub view_id: ViewIdentifier,
    pub syntax: SyntaxDefinition,
    config_table: ConfigTable,
    pub config: Option<BufferConfig>,
    pub path: Option<PathBuf>,
}

pub struct PluginCtx<'a> {
    inner: &'a RpcCtx,
    /// Information about the view initiating this RPC.
    pub view: &'a ViewState,
    pub plugin_id: PluginPid,
}

/// The handler that does low level plugin setup, and then forwards RPC calls
/// to another `Handler` type.
struct BaseHandler<'a, H: 'a> {
    inner: &'a mut H,
    plugin_id: Option<PluginPid>,
    state: Option<ViewState>,
}

/// Abstracts getting data from the peer. This only exists so we can mock it in tests.
pub trait DataSource {
    fn get_data(&self, offset: usize, max_size: usize, rev: u64)
        -> Result<plugin_rpc::GetDataResponse, Error>;
}

impl ViewState {
    fn new(init_info: &plugin_rpc::PluginBufferInfo) -> Self {

        let &plugin_rpc::PluginBufferInfo {
            ref views, ref path, ref syntax, ref config, ..
        } = init_info;

        ViewState {
            view_id: *views.first().unwrap(),
            syntax: *syntax,
            config_table: config.clone(),
            config: serde_json::from_value(Value::Object(config.clone())).unwrap(),
            path: path.as_ref().map(PathBuf::from)
        }
    }

    fn update_config(&mut self, changes: &ConfigTable) {
        for (key, value) in changes.iter() {
            self.config_table.insert(key.to_owned(), value.to_owned());
        }
        let conf = serde_json::from_value(Value::Object(self.config_table.clone()));
        self.config = conf.unwrap();
    }

    fn update_path(&mut self, path: &Path) {
        self.path = Some(path.to_owned())
    }
}

impl<'a> PluginCtx<'a> {
    fn new(inner: &'a RpcCtx, view: &'a ViewState, plugin_id: PluginPid) -> Self {
        PluginCtx { inner, view, plugin_id }
    }

    pub fn add_scopes(&self, scopes: &Vec<Vec<String>>) {
        let params = json!({
            "plugin_id": self.plugin_id,
            "view_id": self.view.view_id,
            "scopes": scopes,
        });
        self.send_rpc_notification("add_scopes", &params);
    }

    pub fn update_spans(&self, start: usize, len: usize, rev: u64, spans: &[plugin_rpc::ScopeSpan]) {
        let params = json!({
            "plugin_id": self.plugin_id,
            "view_id": self.view.view_id,
            "start": start,
            "len": len,
            "rev": rev,
            "spans": spans,
        });
        self.send_rpc_notification("update_spans", &params);
    }

    fn send_rpc_notification(&self, method: &str, params: &Value) {
        self.inner.get_peer().send_rpc_notification(method, params)
    }

    fn send_rpc_request(&self, method: &str, params: &Value) -> Result<Value, xi_rpc::Error> {
        self.inner.get_peer().send_rpc_request(method, params)
    }

    /// Determines whether an incoming request (or notification) is pending. This
    /// is intended to reduce latency for bulk operations done in the background.
    pub fn request_is_pending(&self) -> bool {
        self.inner.get_peer().request_is_pending()
    }

    /// Schedule the idle handler to be run when there are no requests pending.
    pub fn schedule_idle(&mut self, token: usize) {
        self.inner.schedule_idle(token);
    }

    pub fn get_peer(&self) -> &RpcPeer {
        self.inner.get_peer()
    }
}

impl<'a> DataSource for PluginCtx<'a> {
    fn get_data(&self, offset: usize, max_size: usize, rev: u64)
        -> Result<plugin_rpc::GetDataResponse, Error> {
        let params = json!({
            "plugin_id": self.plugin_id,
            "view_id": self.view.view_id,
            "start": offset,
            "unit": "utf8",
            "max_size": max_size,
            "rev": rev,
        });
        let result = self.send_rpc_request("get_data", &params)
            .map_err(|e| Error::RpcError(e))?;
        plugin_rpc::GetDataResponse::deserialize(result)
            .map_err(|_| Error::WrongReturnType)
    }
}

impl<'a, H: 'a> BaseHandler<'a, H> {
    fn new(inner: &'a mut H) -> Self {
        BaseHandler {
            inner: inner,
            plugin_id: None,
            state: None,
        }
    }

    fn expect_state_mut(&mut self) -> &mut ViewState {
        self.state.as_mut()
            .expect("missing state; was plugin init RPC sent?")
    }
}

impl<'a, H: Handler> xi_rpc::Handler for BaseHandler<'a, H> {
    type Notification = plugin_rpc::HostNotification;
    type Request = plugin_rpc::HostRequest;
    fn handle_notification(&mut self, ctx: &RpcCtx, rpc: Self::Notification) {
        use self::plugin_rpc::HostNotification::*;
        // we handle a few RPCs here, updating basic view information
        // before forwarding to the actual handler.
        match rpc {
            // don't forward ping before we're initialized
            Ping( .. ) => { if self.state.is_none() { return } }
            Initialize { ref plugin_id, ref buffer_info } => {
                assert!(self.state.is_none());
                self.state = Some(ViewState::new(buffer_info.first().as_ref().expect("missing buffer info?")));
                self.plugin_id = Some(*plugin_id);
            }

            ConfigChanged { ref changes, .. } =>
                self.expect_state_mut().update_config(changes),

            DidSave { ref path, .. } =>
                self.expect_state_mut().update_path(path),

            TracingConfig {enabled} => {
                use xi_trace;

                if enabled {
                    eprintln!("Enabling tracing in {:?}", self.plugin_id);
                    xi_trace::enable_tracing();
                } else {
                    eprintln!("Disabling tracing in {:?}",  self.plugin_id);
                    xi_trace::disable_tracing();
                }
            }
            _ => (),
        }

        let plugin_ctx = PluginCtx::new(
            ctx, self.state.as_ref().unwrap(), self.plugin_id.unwrap());
        self.inner.handle_notification(plugin_ctx, rpc)
    }

    fn handle_request(&mut self, ctx: &RpcCtx, rpc: Self::Request)
                      -> Result<Value, RemoteError> {
        assert!(self.state.is_some(), "request received before init: {:?}", &rpc);
        let plugin_ctx = PluginCtx::new(
            ctx, self.state.as_ref().unwrap(), self.plugin_id.unwrap());
        self.inner.handle_request(plugin_ctx, rpc)
    }

    fn idle(&mut self, ctx: &RpcCtx, token: usize) {
        let plugin_ctx = PluginCtx::new(
            ctx, self.state.as_ref().unwrap(), self.plugin_id.unwrap());
        self.inner.idle(plugin_ctx, token);
    }
}

pub fn mainloop<H: Handler>(handler: &mut H) -> Result<(), ReadError> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut rpc_looper = RpcLoop::new(stdout);
    let mut my_handler = BaseHandler::new(handler);

    rpc_looper.mainloop(|| stdin.lock(), &mut my_handler)
}
