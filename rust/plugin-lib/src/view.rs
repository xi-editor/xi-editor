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

use serde::Deserialize;
use serde_json::{self, Value};
use std::path::{Path, PathBuf};

use crate::xi_core::plugin_rpc::{
    GetDataResponse, PluginBufferInfo, PluginEdit, ScopeSpan, TextUnit,
};
use crate::xi_core::{BufferConfig, ConfigTable, LanguageId, PluginPid, ViewId};
use xi_core_lib::annotations::AnnotationType;
use xi_core_lib::plugin_rpc::DataSpan;
use xi_rope::interval::IntervalBounds;
use xi_rope::RopeDelta;
use xi_trace::trace_block;

use xi_rpc::RpcPeer;

use super::{Cache, DataSource, Error};

/// A type that acts as a proxy for a remote view. Provides access to
/// a document cache, and implements various methods for querying and modifying
/// view state.
pub struct View<C> {
    pub(crate) cache: C,
    pub(crate) peer: RpcPeer,
    pub(crate) path: Option<PathBuf>,
    pub(crate) config: BufferConfig,
    pub(crate) config_table: ConfigTable,
    plugin_id: PluginPid,
    // TODO: this is only public to avoid changing the syntect impl
    // this should go away with async edits
    pub rev: u64,
    pub undo_group: Option<usize>,
    buf_size: usize,
    pub(crate) view_id: ViewId,
    pub(crate) language_id: LanguageId,
}

impl<C: Cache> View<C> {
    pub(crate) fn new(peer: RpcPeer, plugin_id: PluginPid, info: PluginBufferInfo) -> Self {
        let PluginBufferInfo { views, rev, path, config, buf_size, nb_lines, syntax, .. } = info;

        assert_eq!(views.len(), 1, "assuming single view");
        let view_id = views.first().unwrap().to_owned();
        let path = path.map(PathBuf::from);
        View {
            cache: C::new(buf_size, rev, nb_lines),
            peer,
            config_table: config.clone(),
            config: serde_json::from_value(Value::Object(config)).unwrap(),
            path,
            plugin_id,
            view_id,
            rev,
            undo_group: None,
            buf_size,
            language_id: syntax,
        }
    }

    pub(crate) fn update(
        &mut self,
        delta: Option<&RopeDelta>,
        new_len: usize,
        new_num_lines: usize,
        rev: u64,
        undo_group: Option<usize>,
    ) {
        self.cache.update(delta, new_len, new_num_lines, rev);
        self.rev = rev;
        self.undo_group = undo_group;
        self.buf_size = new_len;
    }

    pub(crate) fn set_language(&mut self, new_language_id: LanguageId) {
        self.language_id = new_language_id;
    }

    //NOTE: (discuss in review) this feels bad, but because we're mutating cache,
    // which we own, we can't just pass in a reference to something else we own;
    // so we create this on each call. The `clone`is only cloning an `Arc`,
    // but we could maybe use a RefCell or something and make this cleaner.
    /// Returns a `FetchCtx`, a thin wrapper around an RpcPeer that implements
    /// the `DataSource` trait and can be used when updating a cache.
    pub(crate) fn make_ctx(&self) -> FetchCtx {
        FetchCtx { view_id: self.view_id, plugin_id: self.plugin_id, peer: self.peer.clone() }
    }

    /// Returns the length of the view's buffer, in bytes.
    pub fn get_buf_size(&self) -> usize {
        self.buf_size
    }

    pub fn get_path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn get_language_id(&self) -> &LanguageId {
        &self.language_id
    }

    pub fn get_config(&self) -> &BufferConfig {
        &self.config
    }

    pub fn get_cache(&mut self) -> &mut C {
        &mut self.cache
    }

    pub fn get_id(&self) -> ViewId {
        self.view_id
    }

    pub fn get_line(&mut self, line_num: usize) -> Result<&str, Error> {
        let ctx = self.make_ctx();
        self.cache.get_line(&ctx, line_num)
    }

    /// Returns a region of the view's buffer.
    pub fn get_region<I: IntervalBounds>(&mut self, interval: I) -> Result<&str, Error> {
        let ctx = self.make_ctx();
        self.cache.get_region(&ctx, interval)
    }

    pub fn get_document(&mut self) -> Result<String, Error> {
        let ctx = self.make_ctx();
        self.cache.get_document(&ctx)
    }

    pub fn offset_of_line(&mut self, line_num: usize) -> Result<usize, Error> {
        let ctx = self.make_ctx();
        self.cache.offset_of_line(&ctx, line_num)
    }

    pub fn line_of_offset(&mut self, offset: usize) -> Result<usize, Error> {
        let ctx = self.make_ctx();
        self.cache.line_of_offset(&ctx, offset)
    }

    pub fn add_scopes(&self, scopes: &[Vec<String>]) {
        let params = json!({
            "plugin_id": self.plugin_id,
            "view_id": self.view_id,
            "scopes": scopes,
        });
        self.peer.send_rpc_notification("add_scopes", &params);
    }

    pub fn edit(
        &self,
        delta: RopeDelta,
        priority: u64,
        after_cursor: bool,
        new_undo_group: bool,
        author: String,
    ) {
        let undo_group = if new_undo_group { None } else { self.undo_group };
        let edit = PluginEdit { rev: self.rev, delta, priority, after_cursor, undo_group, author };
        let params = json!({
            "plugin_id": self.plugin_id,
            "view_id": self.view_id,
            "edit": edit
        });
        self.peer.send_rpc_notification("edit", &params);
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

    pub fn update_annotations(
        &self,
        start: usize,
        len: usize,
        annotation_spans: &[DataSpan],
        annotation_type: &AnnotationType,
    ) {
        let params = json!({
            "plugin_id": self.plugin_id,
            "view_id": self.view_id,
            "start": start,
            "len": len,
            "rev": self.rev,
            "spans": annotation_spans,
            "annotation_type": annotation_type,
        });
        self.peer.send_rpc_notification("update_annotations", &params);
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

    pub fn add_status_item(&self, key: &str, value: &str, alignment: &str) {
        let params = json!({
            "plugin_id": self.plugin_id,
            "view_id": self.view_id,
            "key": key,
            "value": value,
            "alignment": alignment
        });
        self.peer.send_rpc_notification("add_status_item", &params);
    }

    pub fn update_status_item(&self, key: &str, value: &str) {
        let params = json!({
            "plugin_id": self.plugin_id,
            "view_id": self.view_id,
            "key": key,
            "value": value
        });
        self.peer.send_rpc_notification("update_status_item", &params);
    }

    pub fn remove_status_item(&self, key: &str) {
        let params = json!({
            "plugin_id": self.plugin_id,
            "view_id": self.view_id,
            "key": key
        });
        self.peer.send_rpc_notification("remove_status_item", &params);
    }
}

/// A simple wrapper type that acts as a `DataSource`.
pub struct FetchCtx {
    plugin_id: PluginPid,
    view_id: ViewId,
    peer: RpcPeer,
}

impl DataSource for FetchCtx {
    fn get_data(
        &self,
        start: usize,
        unit: TextUnit,
        max_size: usize,
        rev: u64,
    ) -> Result<GetDataResponse, Error> {
        let _t = trace_block("FetchCtx::get_data", &["plugin"]);
        let params = json!({
            "plugin_id": self.plugin_id,
            "view_id": self.view_id,
            "start": start,
            "unit": unit,
            "max_size": max_size,
            "rev": rev,
        });
        let result = self.peer.send_rpc_request("get_data", &params).map_err(Error::RpcError)?;
        GetDataResponse::deserialize(result).map_err(|_| Error::WrongReturnType)
    }
}
