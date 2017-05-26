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

//! A caching layer for xi plugins. Will be split out into its own crate once it's a bit more stable.

use std::path::PathBuf;
use serde_json::Value;

use plugin_base;
use plugin_base::PluginRequest;

pub use plugin_base::{Error, ScopeSpan};

const CHUNK_SIZE: usize = 1024 * 1024;

/// A handler that the plugin needs to instantiate.
pub trait Handler {
    fn initialize(&mut self, ctx: PluginCtx, buf_size: usize);
    fn update(&mut self, ctx: PluginCtx);
    #[allow(unused_variables)]
    fn idle(&mut self, ctx: PluginCtx, token: usize) {}
}

/// The caching state
#[derive(Default)]
struct State {
    buf_size: usize,
    rev: usize,
    cache: Option<String>,
    cache_offset: usize,

    // line iteration state
    line_num: usize,
    offset_of_line: usize,

    syntax: String,
    path: Option<PathBuf>,
}

pub struct PluginCtx<'a> {
    state: &'a mut State,
    peer: plugin_base::PluginCtx<'a>,
}

struct MyHandler<'a, H: 'a> {
    handler: &'a mut H,
    state: State,
}

impl<'a, H: Handler> plugin_base::Handler for MyHandler<'a, H> {
    fn call(&mut self, req: &PluginRequest, peer: plugin_base::PluginCtx) -> Option<Value> {
        let ctx = PluginCtx {
            state: &mut self.state,
            peer: peer,
        };
        match *req {
            PluginRequest::Ping => {
                print_err!("got ping");
                None
            }
            PluginRequest::Initialize(ref init_info) => {
                ctx.state.buf_size = init_info.buf_size;
                ctx.state.rev = init_info.rev;
                ctx.state.syntax = init_info.syntax.clone();
                ctx.state.path = init_info.path.clone().map(|p| PathBuf::from(p));
                self.handler.initialize(ctx, init_info.buf_size);
                None
            }
            PluginRequest::Update { start, end, new_len, rev, edit_type, author, text } => {
                //print_err!("got update notification {:?}", edit_type);
                ctx.state.buf_size = ctx.state.buf_size - (end - start) + new_len;
                ctx.state.rev = rev;
                if let (Some(text), Some(mut cache)) = (text, ctx.state.cache.take()) {
                    let off = ctx.state.cache_offset;
                    if start >= off && start <= off + cache.len() {
                        let tail = if end < off + cache.len() {
                            Some(cache[end - off ..].to_string())
                        } else {
                            None
                        };
                        cache.truncate(start - off);
                        cache.push_str(text);
                        if let Some(tail) = tail {
                            cache.push_str(&tail);
                        }
                        ctx.state.cache = Some(cache);
                    }
                }
                ctx.state.line_num = 0;
                ctx.state.offset_of_line = 0;
                self.handler.update(ctx);
                Some(Value::from(0i32))
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

pub fn mainloop<H: Handler>(handler: &mut H) {
    let mut my_handler = MyHandler {
        handler: handler,
        state: State::default(),
    };
    plugin_base::mainloop(&mut my_handler);
}

impl<'a> PluginCtx<'a> {
    pub fn get_line(&mut self, line_num: usize) -> Result<Option<String>, Error> {
        if line_num != self.state.line_num {
            print_err!("can't handle non-sequential line numbers yet");
            return Ok(None);
        }
        let offset_of_line = self.state.offset_of_line;
        if offset_of_line == self.state.buf_size {
            return Ok(None);
        }
        if self.state.cache.is_none() || offset_of_line < self.state.cache_offset ||
                offset_of_line >= self.state.cache_offset +
                    self.state.cache.as_ref().unwrap().len() {
            self.state.cache = Some(self.peer.get_data(offset_of_line, CHUNK_SIZE,
                self.state.rev)?);
            self.state.cache_offset = offset_of_line;
        }
        loop {
            let offset_in_cache = offset_of_line - self.state.cache_offset;
            match memchr(b'\n', &self.state.cache.as_ref().unwrap().as_bytes()[offset_in_cache..]) {
                None => {
                    let cache_len = self.state.cache.as_ref().unwrap().len();
                    if self.state.cache_offset + cache_len == self.state.buf_size {
                        // incomplete last line
                        let cache = self.state.cache.as_ref().unwrap();
                        let result = String::from(&cache[offset_in_cache..]);
                        self.state.offset_of_line += result.len();
                        self.state.line_num += 1;
                        return Ok(Some(result));
                    }
                    // fetch next chunk
                    let next_offset = self.state.cache_offset + cache_len;
                    let next_chunk = self.peer.get_data(next_offset, CHUNK_SIZE,
                            self.state.rev)?;
                    self.state.cache_offset = offset_of_line;
                    let mut new_cache = String::with_capacity(cache_len - offset_in_cache +
                            next_chunk.len());
                    new_cache.push_str(&self.state.cache.as_ref().unwrap()[offset_in_cache..]);
                    new_cache.push_str(&next_chunk);
                    self.state.cache = Some(new_cache);
                }
                Some(pos) => {
                    let cache = self.state.cache.as_ref().unwrap();
                    let result = String::from(&cache[offset_in_cache .. offset_in_cache + pos + 1]);
                    self.state.offset_of_line += pos + 1;
                    self.state.line_num += 1;
                    return Ok(Some(result));
                }
            }
        }
    }

    pub fn get_path(&self) -> Option<&PathBuf> {
        match self.state.path {
            Some(ref p) => Some(p),
            None => None,
        }
    }

    pub fn add_scopes(&self, scopes: &[String]) {
        self.peer.add_scopes(scopes)
    }

    pub fn update_spans(&self, start: usize, len: usize, spans: &[ScopeSpan]) {
        self.peer.update_spans(start, len, self.state.rev, spans)
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
}

// TODO: use burntsushi memchr, or import this from xi_rope
fn memchr(needle: u8, haystack: &[u8]) -> Option<usize> {
    haystack.iter().position(|&b| b == needle)
}
