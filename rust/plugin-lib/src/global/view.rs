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

use std::path::{Path, PathBuf};
use serde_json::{self, Value};
use serde::Deserialize;

use xi_core::{ViewIdentifier, PluginPid, BufferConfig, ConfigTable};
use xi_core::plugin_rpc::{TextUnit, GetDataResponse, ScopeSpan, PluginBufferInfo};

use xi_rpc::{RpcPeer, RemoteError};
use xi_rope::rope::RopeDelta;

use plugin_base::{Error, DataSource};

pub struct View<C> {
    pub (crate) cache: C,
    peer: RpcPeer,
    pub (crate) path: Option<PathBuf>,
    config: BufferConfig,
    config_table: ConfigTable,
    plugin_id: PluginPid,
    pub (crate) view_id: ViewIdentifier,
}

struct FetchCtx {
    plugin_id: PluginPid,
    view_id: ViewIdentifier,
    peer: RpcPeer,
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
        }
    }

    pub fn get_path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    pub fn get_config(&self) -> &BufferConfig {
        &self.config
    }

    pub fn add_scopes(&self, scopes: &Vec<Vec<String>>) {

    }

    pub fn update_spans(&self, start: usize, len: usize,
                        spans: &[ScopeSpan]) {

    }

    pub fn do_edit(&self) {

    }

    pub fn get_line(&self, line_num: usize) -> Result<&str, Error> {
        let ctx = FetchCtx {
            view_id: self.view_id,
            plugin_id: self.plugin_id,
            peer: self.peer.clone(),
        };
        self.cache.get_line(&ctx, line_num)
    }

    pub fn schedule_idle(&self) {
        let token: usize = self.view_id.into();
        self.peer.schedule_idle(token);
    }
}


/// A cache of a document's contents
pub trait Cache {
    fn new(buf_size: usize, rev: u64, num_lines: usize) -> Self;
    fn get_line<DS>(&self, source: &DS, line_num: usize) -> Result<&str, Error>;
    /// Updates the cache by applying this delta'.
    fn update(&mut self, delta: Option<&RopeDelta>, buf_size: usize,
              num_lines: usize, rev: u64);
    /// Flushes any state held by this cache.
    fn clear(&mut self);
}

pub trait Plugin {
    type Cache: Cache;

    fn update(&mut self, view: &mut View<Self::Cache>, delta: Option<&RopeDelta>)
        -> Result<Value, RemoteError>;
    fn did_save(&mut self, view: &mut View<Self::Cache>, new_path: &Path);
    fn did_close(&self, view: &View<Self::Cache>);
    fn new_view(&mut self, view: &mut View<Self::Cache>);

    /// `view.config` contains the pre-change config
    fn config_changed(&mut self, view: &mut View<Self::Cache>, changes: &ConfigTable);
    fn idle(&mut self) { }
}

impl DataSource for FetchCtx {
    fn get_data(&self, start: usize, unit: TextUnit, max_size: usize, rev: u64)
        -> Result<GetDataResponse, Error> {
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
