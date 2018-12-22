// Copyright 2018 The xi-editor Authors.
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
use crate::xi_core::plugin_rpc::Hover;
use crate::xi_core::plugins::PluginId;
use crate::xi_core::ViewId;
use xi_rpc::{RemoteError, RpcCtx, RpcPeer};

#[derive(Clone)]
pub struct CoreProxy {
    plugin_id: PluginId,
    peer: RpcPeer,
}

impl CoreProxy {
    pub fn new(plugin_id: PluginId, rpc_ctx: &RpcCtx) -> Self {
        CoreProxy { plugin_id, peer: rpc_ctx.get_peer().clone() }
    }

    pub fn add_status_item(&mut self, view_id: ViewId, key: &str, value: &str, alignment: &str) {
        let params = json!({
            "plugin_id": self.plugin_id,
            "view_id": view_id,
            "key": key,
            "value": value,
            "alignment": alignment
        });

        self.peer.send_rpc_notification("add_status_item", &params)
    }

    pub fn update_status_item(&mut self, view_id: ViewId, key: &str, value: &str) {
        let params = json!({
            "plugin_id": self.plugin_id,
            "view_id": view_id,
            "key": key,
            "value": value
        });

        self.peer.send_rpc_notification("update_status_item", &params)
    }

    pub fn remove_status_item(&mut self, view_id: ViewId, key: &str) {
        let params = json!({
            "plugin_id": self.plugin_id,
            "view_id": view_id,
            "key": key
        });

        self.peer.send_rpc_notification("remove_status_item", &params)
    }

    pub fn display_hover(
        &mut self,
        view_id: ViewId,
        request_id: usize,
        result: &Result<Hover, RemoteError>,
    ) {
        let params = json!({
            "plugin_id": self.plugin_id,
            "request_id": request_id,
            "result": result,
            "view_id": view_id
        });

        self.peer.send_rpc_notification("show_hover", &params);
    }

    pub fn schedule_idle(&mut self, view_id: ViewId) {
        let token: usize = view_id.into();
        self.peer.schedule_idle(token);
    }
}
