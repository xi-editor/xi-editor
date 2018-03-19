use std::path::{Path, PathBuf};
use serde_json::Value;

use xi_core::{ViewIdentifier, PluginPid, BufferConfig, ConfigTable, plugin_rpc};
use xi_rpc::RpcPeer;
use xi_rope::rope::{RopeDelta, LinesMetric};

use plugin_base::{Error, DataSource};

pub struct View<C> {
    cache: C,
    peer: RpcPeer,
    path: Option<PathBuf>,
    config: BufferConfig,
    config_table: ConfigTable,
    plugin_id: PluginPid,
    view_id: ViewIdentifier,
}

struct FetchCtx {
    plugin_id: PluginPid,
    view_id: ViewIdentifier,
    peer: RpcPeer,
}

impl<C: Cache> View<C> {
    fn get_path(&self) -> Option<&Path> {
        unimplemented!()
    }

    fn get_config(&self) -> &BufferConfig {
        unimplemented!()
    }

    pub fn add_scopes(&self, scopes: &Vec<Vec<String>>) {

    }

    pub fn update_spans(&self, start: usize, len: usize,
                        spans: &[plugin_rpc::ScopeSpan]) {

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
}


/// A cache of a document's contents
pub trait Cache {
    fn new(buf_size: usize, rev: u64) -> Self;
    fn get_line<DS>(&self, source: &DS, line_num: usize) -> Result<&str, Error>;
    /// Updates the cache by applying this delta'.
    fn update(&mut self, delta: &RopeDelta, buf_size: usize, rev: u64);
    /// Flushes any state held by this cache.
    fn clear(&mut self);
}

pub trait Plugin {
    type Cache: Cache;

    fn initialize(&mut self) {

    }

    fn update(&mut self, view: &View<Self::Cache>, delta: RopeDelta, rev: usize) {

    }

    fn did_save(&mut self, view: &View<Self::Cache>) {

    }

    fn did_close(&mut self, view: &View<Self::Cache>) {

    }

    fn new_view(&mut self, view: &View<Self::Cache>) {

    }

    /// `view.config` contains the pre-change config
    fn config_changed(&mut self, view: &View<Self::Cache>, changes: &ConfigTable) {

    }

    fn idle(&mut self) {

    }
}

impl DataSource for FetchCtx {
    fn get_data(&self, offset: usize, max_size: usize, rev: u64) -> Result<String, Error> {
        let params = json!({
            "plugin_id": self.plugin_id,
            "view_id": self.view_id,
            "offset": offset,
            "max_size": max_size,
            "rev": rev,
        });
        let result = self.peer.send_rpc_request("get_data", &params);
        match result {
            Ok(Value::String(s)) => Ok(s),
            Ok(_) => Err(Error::WrongReturnType),
            Err(err) => Err(Error::RpcError(err)),
        }
    }
}
