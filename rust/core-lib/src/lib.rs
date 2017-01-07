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

//! The main library for xi-core.

extern crate serde;
extern crate serde_json;
extern crate time;

use std::io::Write;

use serde_json::Value;

#[macro_use]
mod macros;

pub mod tabs;
pub mod rpc;
mod editor;
mod view;
mod linewrap;
mod run_plugin;

use tabs::Tabs;
use rpc::Request;

extern crate xi_rope;
extern crate xi_unicode;
extern crate xi_rpc;

use xi_rpc::{RpcPeer, RpcCtx, Handler};

pub type MainPeer<W> = RpcPeer<W>;

pub struct MainState<W: Write> {
    tabs: Tabs<W>,
}

impl<W: Write + Send + 'static> MainState<W> {
    pub fn new() -> Self {
        MainState {
            tabs: Tabs::new(),
        }
    }
}

impl<W: Write + Send + 'static> Handler<W> for MainState<W> {
    fn handle_notification(&mut self, ctx: RpcCtx<W>, method: &str, params: &Value) {
        match Request::from_json(method, params) {
            Ok(req) => {
                let _ = self.handle_req(req, ctx.get_peer());
                // TODO: should check None
            }
            Err(e) => print_err!("Error {} decoding RPC request {}", e, method)
        }
    }

    fn handle_request(&mut self, ctx: RpcCtx<W>, method: &str, params: &Value) ->
        Result<Value, Value> {
        match Request::from_json(method, params) {
            Ok(req) => {
                let result = self.handle_req(req, ctx.get_peer());
                result.ok_or_else(|| Value::String("return value missing".to_string()))
            }
            Err(e) => {
                print_err!("Error {} decoding RPC request {}", e, method);
                Err(Value::String("error decoding request".to_string()))
            }
        }
    }
}

impl<W: Write + Send + 'static> MainState<W> {
    fn handle_req(&mut self, request: Request, rpc_peer: &MainPeer<W>) ->
        Option<Value> {
        match request {
            Request::TabCommand { tab_command } => self.tabs.do_rpc(tab_command, rpc_peer)
        }
    }
}
