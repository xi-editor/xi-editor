// Copyright 2018 Google Inc. All rights reserved.
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

use std::path::{PathBuf, Path};
use serde_json::{self, Value};
use serde::Deserialize;

use xi_core::{ViewIdentifier, PluginPid, BufferConfig, ConfigTable};
use xi_core::plugin_rpc::{TextUnit, GetDataResponse, ScopeSpan, PluginBufferInfo};
use xi_rope::rope::RopeDelta;
use xi_trace::trace_block;

use xi_rpc::RpcPeer;

use plugin_base::{Error, DataSource};

use global::Cache;

/// A type that acts as a proxy for a remote view. Provides access to
/// a document cache, and implements various methods for querying and modifying
/// view state.
pub struct View<C> {
    pub (crate) cache: C,
    pub (crate) peer: RpcPeer,
    pub (crate) path: Option<PathBuf>,
    pub (crate) config: BufferConfig,
    pub (crate) config_table: ConfigTable,
    plugin_id: PluginPid,
    pub (crate) rev: u64,
    pub (crate) view_id: ViewIdentifier,
}

impl<C: Cache> View<C> {
    pub (crate) fn new(peer: RpcPeer, plugin_id: PluginPid,
                       info: PluginBufferInfo) -> Self {
        let PluginBufferInfo {
            views, rev, path, config, buf_size, nb_lines, ..
        } = info;

        assert_eq!(views.len(), 1, "assuming single view");
        let view_id = views.first().unwrap().to_owned();
        let path = path.map(PathBuf::from);
        View {
            cache: C::new(buf_size, rev, nb_lines),
            peer: peer,
            config_table: config.clone(),
            config: serde_json::from_value(Value::Object(config)).unwrap(),
            path: path,
            plugin_id: plugin_id,
            view_id: view_id,
            rev: rev,
        }
    }

    pub (crate) fn update(&mut self, delta: Option<&RopeDelta>, new_len: usize,
                       new_num_lines: usize, rev: u64) {
        self.cache.update(delta, new_len, new_num_lines, rev);
        self.rev = rev;
    }

    //NOTE: (discuss in review) this feels bad, but because we're mutating cache,
    // which we own, we can't just pass in a reference to something else we own;
    // so we create this on each call. The `clone`is only cloning an `Arc`,
    // but we could maybe use a RefCell or something and make this cleaner.
    /// Returns a `FetchCtx`, a thin wrapper around an RpcPeer that implements
    /// the `DataSource` trait and can be used when updating a cache.
    pub (crate) fn make_ctx(&self) -> FetchCtx {
        FetchCtx {
            view_id: self.view_id,
            plugin_id: self.plugin_id,
            peer: self.peer.clone(),
        }
    }

    pub fn get_path(&self) -> Option<&Path> {
        self.path.as_ref().map(PathBuf::as_path)
    }

    pub fn get_config(&self) -> &BufferConfig {
        &self.config
    }

    pub fn get_cache(&mut self) -> &mut C {
        &mut self.cache
    }

    pub fn get_id(&self) -> ViewIdentifier {
        self.view_id.clone()
    }

    pub fn get_line(&mut self, line_num: usize) -> Result<&str, Error> {
        let ctx = self.make_ctx();
        self.cache.get_line(&ctx, line_num)
    }

    pub fn add_scopes(&self, scopes: &Vec<Vec<String>>) {
        let params = json!({
            "plugin_id": self.plugin_id,
            "view_id": self.view_id,
            "scopes": scopes,
        });
        self.peer.send_rpc_notification("add_scopes", &params);
    }

    pub fn update_spans(&self, start: usize, len: usize, spans: &[ScopeSpan]) {
        let params = json!({
            "plugin_id": self.plugin_id,
            "view_id": self.view_id,
            "start": start,
            "len": len,
            "rev": self.rev,
            "spans": spans,
        });
        self.peer.send_rpc_notification("update_spans", &params);
    }

    pub fn schedule_idle(&self) {
        let token: usize = self.view_id.into();
        self.peer.schedule_idle(token);
    }

    /// Returns `true` if an incoming RPC is pending. This is intended
    /// to reduce latency for bulk operations done in the background.
    pub fn request_is_pending(&self) -> bool {
        self.peer.request_is_pending()
    }

}

/// A simple wrapper type that acts as a `DataSource`.
pub struct FetchCtx {
    plugin_id: PluginPid,
    view_id: ViewIdentifier,
    peer: RpcPeer,
}

impl DataSource for FetchCtx {
    fn get_data(&self, start: usize, unit: TextUnit, max_size: usize, rev: u64)
        -> Result<GetDataResponse, Error> {
        let _t = trace_block("FetchCtx::get_data", &["plugin"]);
        let params = json!({
            "plugin_id": self.plugin_id,
            "view_id": self.view_id,
            "start": start,
            "unit": unit,
            "max_size": max_size,
            "rev": rev,
        });
        let result = self.peer.send_rpc_request("get_data", &params)
            .map_err(|e| Error::RpcError(e))?;
        GetDataResponse::deserialize(result)
            .map_err(|_| Error::WrongReturnType)
    }
}
