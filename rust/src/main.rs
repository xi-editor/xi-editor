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

extern crate serde;
extern crate serde_json;
extern crate time;

use std::io;
use std::io::Write;

use serde_json::Value;

#[macro_use]
mod macros;

mod tabs;
mod editor;
mod view;
mod linewrap;
mod rpc;
mod rpc_peer;
mod run_plugin;

use tabs::Tabs;
use rpc::Request;
use rpc_peer::{RpcPeer, RpcWriter};

extern crate xi_rope;
extern crate xi_unicode;

pub type MainPeer<'a> = RpcWriter<io::Stdout>;

fn handle_req<'a>(request: Request, tabs: &mut Tabs, rpc_peer: &MainPeer<'a>) -> Option<Value> {
    match request {
        Request::TabCommand { tab_command } => tabs.do_rpc(tab_command, rpc_peer)
    }
}

fn main() {
    let mut tabs = Tabs::new();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut rpc_peer = RpcPeer::new(stdin.lock(), stdout);
    let rpc_writer = rpc_peer.get_writer();

    rpc_peer.mainloop(|method, params| {
        match Request::from_json(method, params) {
            Ok(req) => handle_req(req, &mut tabs, &rpc_writer),
            Err(e) => {
                print_err!("Error {} decoding RPC request {}", e, method);
                None
            }
        }
    });
}
