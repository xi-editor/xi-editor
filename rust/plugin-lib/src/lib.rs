// Copyright 2017 The xi-editor Authors.
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

//! The library base for implementing xi-editor plugins.
extern crate xi_core_lib as xi_core;
extern crate xi_rope;
extern crate xi_rpc;
extern crate xi_trace;
#[macro_use]
extern crate serde_json;
extern crate bytecount;
extern crate memchr;
extern crate rand;
extern crate serde;

#[macro_use]
extern crate log;

mod base_cache;
mod core_proxy;
mod dispatch;
mod state_cache;
mod view;

use std::io;
use std::path::Path;

use crate::xi_core::plugin_rpc::{GetDataResponse, TextUnit};
use crate::xi_core::{ConfigTable, LanguageId};
use serde_json::Value;
use xi_rope::interval::IntervalBounds;
use xi_rope::RopeDelta;
use xi_rpc::{ReadError, RpcLoop};

use self::dispatch::Dispatcher;

pub use crate::base_cache::ChunkCache;
pub use crate::core_proxy::CoreProxy;
pub use crate::state_cache::StateCache;
pub use crate::view::View;
pub use crate::xi_core::plugin_rpc::{Hover, Range};

/// Abstracts getting data from the peer. Mainly exists for mocking in tests.
pub trait DataSource {
    fn get_data(
        &self,
        start: usize,
        unit: TextUnit,
        max_size: usize,
        rev: u64,
    ) -> Result<GetDataResponse, Error>;
}

/// A generic interface for types that cache a remote document.
///
/// In general, users of this library should not need to implement this trait;
/// we provide two concrete Cache implementations, [`ChunkCache`] and
/// [`StateCache`]. If however a plugin's particular needs are not met by
/// those implementations, a user may choose to implement their own.
///
/// [`ChunkCache`]: ../base_cache/struct.ChunkCache.html
/// [`StateCache`]: ../state_cache/struct.StateCache.html
pub trait Cache {
    /// Create a new instance of this type; instances are created automatically
    /// as relevant views are added.
    fn new(buf_size: usize, rev: u64, num_lines: usize) -> Self;
    /// Returns the line at `line_num` (zero-indexed). Returns an `Err(_)` if
    /// there is a problem connecting to the peer, or if the requested line
    /// is out of bounds.
    ///
    /// The `source` argument is some type that implements [`DataSource`]; in
    /// the general case this is backed by the remote peer.
    ///
    /// [`DataSource`]: trait.DataSource.html
    fn get_line<DS: DataSource>(&mut self, source: &DS, line_num: usize) -> Result<&str, Error>;

    /// Returns the specified region of the buffer. Returns an `Err(_)` if
    /// there is a problem connecting to the peer, or if the requested line
    /// is out of bounds.
    ///
    /// The `source` argument is some type that implements [`DataSource`]; in
    /// the general case this is backed by the remote peer.
    ///
    /// [`DataSource`]: trait.DataSource.html
    fn get_region<DS, I>(&mut self, source: &DS, interval: I) -> Result<&str, Error>
    where
        DS: DataSource,
        I: IntervalBounds;

    /// Returns the entire contents of the remote document, fetching as needed.
    fn get_document<DS: DataSource>(&mut self, source: &DS) -> Result<String, Error>;

    /// Returns the offset of the line at `line_num`, zero-indexed, fetching
    /// data from `source` if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if `line_num` is greater than the total number of lines
    /// in the document, or if there is a problem communicating with `source`.
    fn offset_of_line<DS: DataSource>(
        &mut self,
        source: &DS,
        line_num: usize,
    ) -> Result<usize, Error>;
    /// Returns the index of the line containing `offset`, fetching
    /// data from `source` if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if `offset` is greater than the total length of
    /// the document, or if there is a problem communicating with `source`.
    fn line_of_offset<DS: DataSource>(
        &mut self,
        source: &DS,
        offset: usize,
    ) -> Result<usize, Error>;
    /// Updates the cache by applying this delta.
    fn update(&mut self, delta: Option<&RopeDelta>, buf_size: usize, num_lines: usize, rev: u64);
    /// Flushes any state held by this cache.
    fn clear(&mut self);
}

/// An interface for plugins.
///
/// Users of this library must implement this trait for some type.
pub trait Plugin {
    type Cache: Cache;

    /// Called when the Plugin is initialized. The plugin receives CoreProxy
    /// object that is a wrapper around the RPC Peer and can be used to call
    /// related methods on the Core in a type-safe manner.
    #[allow(unused_variables)]
    fn initialize(&mut self, core: CoreProxy) {}

    /// Called when an edit has occurred in the remote view. If the plugin wishes
    /// to add its own edit, it must do so using asynchronously via the edit notification.
    fn update(
        &mut self,
        view: &mut View<Self::Cache>,
        delta: Option<&RopeDelta>,
        edit_type: String,
        author: String,
    );
    /// Called when a buffer has been saved to disk. The buffer's previous
    /// path, if one existed, is passed as `old_path`.
    fn did_save(&mut self, view: &mut View<Self::Cache>, old_path: Option<&Path>);
    /// Called when a view has been closed. By the time this message is received,
    /// It is possible to send messages to this view. The plugin may wish to
    /// perform cleanup, however.
    fn did_close(&mut self, view: &View<Self::Cache>);
    /// Called when there is a new view that this buffer is interested in.
    /// This is called once per view, and is paired with a call to
    /// `Plugin::did_close` when the view is closed.
    fn new_view(&mut self, view: &mut View<Self::Cache>);

    /// Called when a config option has changed for this view. `changes`
    /// is a map of keys/values that have changed; previous values are available
    /// in the existing config, accessible through `view.get_config()`.
    fn config_changed(&mut self, view: &mut View<Self::Cache>, changes: &ConfigTable);

    /// Called when syntax language has changed for this view.
    /// New language is available in the `view`, and old language is available in `old_lang`.
    #[allow(unused_variables)]
    fn language_changed(&mut self, view: &mut View<Self::Cache>, old_lang: LanguageId) {}

    /// Called with a custom command.
    #[allow(unused_variables)]
    fn custom_command(&mut self, view: &mut View<Self::Cache>, method: &str, params: Value) {}

    /// Called when the runloop is idle, if the plugin has previously
    /// asked to be scheduled via `View::schedule_idle()`. Plugins that
    /// are doing things like full document analysis can use this mechanism
    /// to perform their work incrementally while remaining responsive.
    #[allow(unused_variables)]
    fn idle(&mut self, view: &mut View<Self::Cache>) {}

    /// Language Plugins specific methods

    #[allow(unused_variables)]
    fn get_hover(&mut self, view: &mut View<Self::Cache>, request_id: usize, position: usize) {}
}

#[derive(Debug)]
pub enum Error {
    RpcError(xi_rpc::Error),
    WrongReturnType,
    BadRequest,
    PeerDisconnect,
    // Just used in tests
    Other(String),
}

/// Run `plugin` until it exits, blocking the current thread.
pub fn mainloop<P: Plugin>(plugin: &mut P) -> Result<(), ReadError> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut rpc_looper = RpcLoop::new(stdout);
    let mut dispatcher = Dispatcher::new(plugin);

    rpc_looper.mainloop(|| stdin.lock(), &mut dispatcher)
}
