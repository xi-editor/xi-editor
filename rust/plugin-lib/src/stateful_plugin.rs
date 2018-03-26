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

use serde_json::Value;

use xi_core::{plugin_rpc, BufferConfig};
use xi_rpc::{RemoteError, ReadError};
use xi_rope::rope::RopeDelta;

use state_cache::StateCache;
use base_cache::ChunkCache;
use global::Cache;
pub use plugin_base::{self, Error, ViewState, DataSource};

/// A handler that the plugin needs to instantiate.
pub trait Plugin {
    type State: Default + Clone;

    fn initialize(&mut self, ctx: PluginCtx<Self::State>, buf_size: usize);
    fn update(&mut self, ctx: PluginCtx<Self::State>, rev: usize,
              delta: Option<RopeDelta>) -> Option<Value>;
    fn did_save(&mut self, ctx: PluginCtx<Self::State>);
    #[allow(unused_variables)]
    fn idle(&mut self, ctx: PluginCtx<Self::State>, token: usize) {}
}

pub struct PluginCtx<'a, S: 'a> {
    state: &'a mut StateCache<S>,
    peer: plugin_base::PluginCtx<'a>,
}

struct CacheHandler<'a, P: Plugin + 'a> {
    handler: &'a mut P,
    state: StateCache<P::State>,
}

impl<'a, P: Plugin> plugin_base::Handler for CacheHandler<'a, P> {
    fn handle_notification(&mut self, ctx: plugin_base::PluginCtx,
                           rpc: plugin_rpc::HostNotification) {
        use self::plugin_rpc::HostNotification::*;
        let ctx = PluginCtx {
            state: &mut self.state,
            peer: ctx,
        };
        match rpc {
            Ping( .. ) => (),
            Initialize { mut buffer_info, .. } => {
                let info = buffer_info.remove(0);
                ctx.do_initialize(info, self.handler);
            }
            // TODO: add this to handler
            ConfigChanged { .. } => (),
            DidSave { .. } => ctx.do_did_save(self.handler),
            NewBuffer { .. } | DidClose { .. } => eprintln!("Rust plugin lib \
            does not support global plugins"),
            //TODO: figure out shutdown
            Shutdown( .. ) | TracingConfig{ .. } => (),
        }
    }

    fn handle_request(&mut self, ctx: plugin_base::PluginCtx,
                      rpc: plugin_rpc::HostRequest)
                      -> Result<Value, RemoteError> {
        use self::plugin_rpc::HostRequest::*;
        let ctx = PluginCtx {
            state: &mut self.state,
            peer: ctx,
        };
        match rpc {
            Update(params) => Ok(ctx.do_update(params, self.handler)),
            CollectTrace( .. ) => {
                use xi_trace;
                use xi_trace_dump::*;

                let samples = xi_trace::samples_cloned_unsorted();
                let serialized_result = chrome_trace::to_value(
                    &samples, chrome_trace::OutputFormat::JsonArray);
                let serialized = serialized_result.map_err(|e| RemoteError::Custom {
                    code: 0,
                    message: format!("{:?}", e),
                    data: None
                })?;
                Ok(serialized)
            }
        }
    }

    fn idle(&mut self, peer: plugin_base::PluginCtx, token: usize) {
        let ctx = PluginCtx {
            state: &mut self.state,
            peer: peer,
        };
        self.handler.idle(ctx, token);
    }
}

impl<'a, S: Default + Clone> PluginCtx<'a, S> {
    fn do_initialize<P>(self, init_info: plugin_rpc::PluginBufferInfo, handler: &mut P)
        where P: Plugin<State = S>
    {

        self.state.buf_cache = ChunkCache::new(init_info.buf_size,
                                               init_info.rev,
                                               init_info.nb_lines);
        self.state.truncate_frontier(0);
        handler.initialize(self, init_info.buf_size);
    }

    fn do_did_save<P: Plugin<State = S>>(self, handler: &mut P) {
        handler.did_save(self);
    }

    fn do_update<P>(self, update: plugin_rpc::PluginUpdate, handler: &mut P) -> Value
        where P: Plugin<State = S>
    {
        let plugin_rpc::PluginUpdate { delta, new_len, rev, new_line_count, .. } = update;
        // update our own state before updating buf_cache
        self.state.update(delta.as_ref(), new_len, new_line_count, rev);
        handler.update(self, rev as usize, delta)
            .unwrap_or(Value::from(0i32))
    }

    /// Provides access to the view state, which contains information about
    /// config options, path, etc.
    pub fn get_view(&self) -> &ViewState {
        &self.peer.view
    }

    pub fn get_config(&self) -> &BufferConfig {
        &self.peer.view.config
    }

    pub fn get_buf_size(&self) -> usize {
        self.state.buf_cache.buf_size
    }

    pub fn add_scopes(&self, scopes: &Vec<Vec<String>>) {
        self.peer.add_scopes(scopes)
    }

    pub fn update_spans(&self, start: usize, len: usize,
                        spans: &[plugin_rpc::ScopeSpan]) {
        self.peer.update_spans(start, len, self.state.buf_cache.rev, spans)
    }

    /// Determines whether an incoming request (or notification) is pending. This
    /// is intended to reduce latency for bulk operations done in the background.
    pub fn request_is_pending(&self) -> bool {
        self.peer.request_is_pending()
    }

    /// Schedule the idle handler to be run when there are no requests pending.
    pub fn schedule_idle(&mut self, token: usize) {
        self.peer.schedule_idle(token);
    }

    // re-expose methods implemented in the state cache

    pub fn get_frontier(&self) -> Option<usize> {
        self.state.get_frontier()
    }

    pub fn get_prev(&self, line_num: usize) -> (usize, usize, S) {
        self.state.get_prev(line_num)
    }

    pub fn get_line(&mut self, line_num: usize) -> Result<&str, Error> {
        self.state.get_line(&self.peer, line_num)
    }

    pub fn get(&self, line_num: usize) -> Option<&S> {
        self.state.get(line_num)
    }

    pub fn set(&mut self, line_num: usize, s: S) {
        self.state.set(&self.peer, line_num, s)
    }

    pub fn update_frontier(&mut self, new_frontier: usize) {
        self.state.update_frontier(new_frontier)
    }

    pub fn close_frontier(&mut self) {
        self.state.close_frontier()
    }

    pub fn reset(&mut self) {
        self.state.reset()
    }

    pub fn find_offset(&self, offset: usize) -> Result<usize, usize> {
        self.state.find_offset(offset)
    }
}

pub fn mainloop<P: Plugin>(handler: &mut P) -> Result<(), ReadError>  {
    let mut my_handler = CacheHandler {
        handler: handler,
        state: StateCache::default(),
    };
    plugin_base::mainloop(&mut my_handler)
}
