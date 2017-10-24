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

use serde_json::Value;

use xi_core::{ViewIdentifier, PluginPid, plugin_rpc};
use xi_rpc::{self, RpcLoop, RpcCtx, RemoteError, ReadError};

// TODO: avoid duplicating this in every crate
macro_rules! print_err {
    ($($arg:tt)*) => (
        {
            use std::io::prelude::*;
            if let Err(e) = write!(&mut ::std::io::stderr(), "{}\n", format_args!($($arg)*)) {
                panic!("Failed to write to stderr.\
                    \nOriginal error output: {}\
                    \nSecondary error writing to stderr: {}", format!($($arg)*), e);
            }
        }
    )
}

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

pub struct PluginCtx<'a>(&'a RpcCtx);

impl<'a> PluginCtx<'a> {
    pub fn get_data(&self, plugin_id: PluginPid, view_id: &ViewIdentifier, offset: usize,
                    max_size: usize, rev: u64) -> Result<String, Error> {
        let params = json!({
            "plugin_id": plugin_id,
            "view_id": view_id,
            "offset": offset,
            "max_size": max_size,
            "rev": rev,
        });
        let result = self.send_rpc_request("get_data", &params);
        match result {
            Ok(Value::String(s)) => Ok(s),
            Ok(_) => Err(Error::WrongReturnType),
            Err(err) => Err(Error::RpcError(err)),
        }
    }

    pub fn add_scopes(&self, plugin_id: PluginPid, view_id: &ViewIdentifier, scopes: &Vec<Vec<String>>) {
        let params = json!({
            "plugin_id": plugin_id,
            "view_id": view_id,
            "scopes": scopes,
        });
        self.send_rpc_notification("add_scopes", &params);
    }

    pub fn update_spans(&self, plugin_id: PluginPid, view_id: &ViewIdentifier, start: usize, len: usize, rev: u64, spans: &[plugin_rpc::ScopeSpan]) {
        let params = json!({
            "plugin_id": plugin_id,
            "view_id": view_id,
            "start": start,
            "len": len,
            "rev": rev,
            "spans": spans,
        });
        self.send_rpc_notification("update_spans", &params);
    }

    fn send_rpc_notification(&self, method: &str, params: &Value) {
        self.0.get_peer().send_rpc_notification(method, params)
    }

    fn send_rpc_request(&self, method: &str, params: &Value) -> Result<Value, xi_rpc::Error> {
        self.0.get_peer().send_rpc_request(method, params)
    }

    /// Determines whether an incoming request (or notification) is pending. This
    /// is intended to reduce latency for bulk operations done in the background.
    pub fn request_is_pending(&self) -> bool {
        self.0.get_peer().request_is_pending()
    }

    /// Schedule the idle handler to be run when there are no requests pending.
    pub fn schedule_idle(&mut self, token: usize) {
        self.0.schedule_idle(token);
    }
}

struct MyHandler<'a, H: 'a>(&'a mut H);

impl<'a, H: Handler> xi_rpc::Handler for MyHandler<'a, H> {
    type Notification = plugin_rpc::HostNotification;
    type Request = plugin_rpc::HostRequest;
    fn handle_notification(&mut self, ctx: &RpcCtx, rpc: Self::Notification) {
        self.0.handle_notification(PluginCtx(ctx), rpc)
    }

    fn handle_request(&mut self, ctx: &RpcCtx, rpc: Self::Request)
                      -> Result<Value, RemoteError> {

        self.0.handle_request(PluginCtx(ctx), rpc)
    }

    fn idle(&mut self, ctx: &RpcCtx, token: usize) {
        self.0.idle(PluginCtx(ctx), token);
    }
}

pub fn mainloop<H: Handler>(handler: &mut H) -> Result<(), ReadError> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut rpc_looper = RpcLoop::new(stdout);
    let mut my_handler = MyHandler(handler);

    rpc_looper.mainloop(|| stdin.lock(), &mut my_handler)
}
