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

mod view;
mod dispatch;

use std::io;
use std::path::Path;

use xi_rpc::{RpcLoop, ReadError};
use xi_rope::rope::RopeDelta;
use xi_core::ConfigTable;
use xi_core::plugin_rpc::PluginEdit;

use plugin_base::{Error, DataSource};

pub use self::view::View;
pub use self::dispatch::Dispatcher;

/// A generic interface for types that cache a remote document.
///
/// In general, users of this library should not need to implement this trait;
/// we provide two concrete Cache implementations, [`ChunkCache`] and
/// [`StateCache`]. If however a plugin's particular needs are not met by
/// those implementations, a user may choose to implement their own.
///
/// [`ChunkCache`]: struct.ChunkCache.html
/// [`StateCache`]: struct.StateCache.html

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
    fn get_line<DS>(&mut self, source: &DS, line_num: usize) -> Result<&str, Error>
        where DS: DataSource;
    /// Updates the cache by applying this delta.
    fn update(&mut self, delta: Option<&RopeDelta>, buf_size: usize,
              num_lines: usize, rev: u64);
    /// Flushes any state held by this cache.
    fn clear(&mut self);
}

/// An interface for plugins.
///
/// Users of this library must implement this trait for some type.
pub trait Plugin {
    type Cache: Cache;

    //TODO: async edits only; this is here for feature paritiy during initial hacking
    /// Called when an edit has occured in the remote view. If the plugin wishes
    /// to add its own edit, it may return `Some(edit)`.
    fn update(&mut self, view: &mut View<Self::Cache>, delta: Option<&RopeDelta>,
              edit_type: String, author: String) -> Option<PluginEdit>;
    /// Called when a buffer has been saved to disk. The buffer's previous
    /// path, if one existed, is available through `view.get_path()`.
    fn did_save(&mut self, view: &mut View<Self::Cache>, new_path: &Path);
    /// Called when a view has been closed. By the time this message is received,
    /// It is possible to send messages to this view. The plugin may wish to
    /// perform cleanup, however.
    fn did_close(&self, view: &View<Self::Cache>);
    /// Called when there is a new view that this buffer is interested in.
    /// This is called once per view, and is paired with a call to
    /// `Plugin::did_close` when the view is closed.
    fn new_view(&mut self, view: &mut View<Self::Cache>);

    /// Called when a config option has changed for this view. `changes`
    /// is a map of keys/values that have changed; previous values are available
    /// in the existing config, accessible through `view.get_config()`.
    fn config_changed(&mut self, view: &mut View<Self::Cache>, changes: &ConfigTable);

    /// Called when the runloop is idle, if the plugin has prevoiusly
    /// asked to be scheduled via `View::schedule_idle()`. Plugins that
    /// are doing things like full document analysis can use this mechanism
    /// to perform their work incrementally while remaining responsive.
    #[allow(unused_variables)]
    fn idle(&mut self, view: &mut View<Self::Cache>) { }
}

//TODO: docs, including an example
pub fn mainloop<P: Plugin>(plugin: &mut P) -> Result<(), ReadError> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut rpc_looper = RpcLoop::new(stdout);
    let mut dispatcher = Dispatcher::new(plugin);

    rpc_looper.mainloop(|| stdin.lock(), &mut dispatcher)
}
