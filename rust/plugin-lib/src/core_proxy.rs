// Copyright 2018 Google LLC
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

//! A proxy for the methods on Core

use xi_core::internal::plugins::PluginId;
use xi_core::ViewId;
use xi_rpc::{RpcCtx, RpcPeer};
use xi_core::plugin_rpc::{HoverResult, DefinitionResult};

#[derive(Clone)]
pub struct CoreProxy {
    plugin_id: PluginId, 
    peer: RpcPeer
}


impl CoreProxy {

    pub fn new(plugin_id: PluginId, rpc_ctx: &RpcCtx) -> Self {
        CoreProxy {
            plugin_id,
            peer: rpc_ctx.get_peer().clone()
        }
    }

    pub fn display_hover_result(&mut self, view_id: ViewId, request_id: usize, result: Option<HoverResult>, rev: u64) {

        let params = json!({
            "plugin_id": self.plugin_id,
            "rev": rev,
            "request_id": request_id,
            "result": result,
            "view_id": view_id
        });

        self.peer.send_rpc_notification("hover_result", &params);
    }

    pub fn display_definition(&mut self, view_id: ViewId, request_id: usize, reuslt: Option<DefinitionResult>, rev: u64) {

    }
}