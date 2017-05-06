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
#[macro_use]
extern crate serde_json;
#[macro_use]
extern crate serde_derive;
extern crate time;
extern crate parking_lot;

use std::io::Write;

use serde_json::Value;

#[macro_use]
mod macros;

pub mod rpc;

/// Internal data structures and logic.
///
/// These internals are not part of the public API (for the purpose of binding to
/// a front-end), but are exposed here, largely so they appear in documentation.
#[path=""]
pub mod internal {
    pub mod tabs;
    pub mod editor;
    pub mod view;
    pub mod linewrap;
    pub mod plugins;
    pub mod styles;
    pub mod word_boundaries;
    pub mod index_set;
    pub mod selection;
    pub mod movement;
}

use internal::tabs;
use internal::editor;
use internal::view;
use internal::linewrap;
use internal::plugins;
use internal::styles;
use internal::word_boundaries;
use internal::index_set;
use internal::selection;
use internal::movement;

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

    fn handle_request(&mut self, mut ctx: RpcCtx<W>, method: &str, params: &Value) ->
        Result<Value, Value> {
        match Request::from_json(method, params) {
            Ok(req) => {
                let result = self.handle_req(req, ctx.get_peer());
                // Schedule the idle handler to send the render the cursor for new
                // empty buffers. TODO: move this into the new_tab logic.
                ctx.schedule_idle(0);
                result.ok_or_else(|| Value::String("return value missing".to_string()))
            }
            Err(e) => {
                print_err!("Error {} decoding RPC request {}", e, method);
                Err(Value::String("error decoding request".to_string()))
            }
        }
    }

    fn idle(&mut self, _ctx: RpcCtx<W>, _token: usize) {
        self.tabs.handle_idle();
    }
}

impl<W: Write + Send + 'static> MainState<W> {
    fn handle_req(&mut self, request: Request, rpc_peer: &MainPeer<W>) ->
        Option<Value> {
        match request {
            Request::CoreCommand { core_command } => self.tabs.do_rpc(core_command, rpc_peer)
        }
    }
}
