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

use serde_json::Value;

use plugin_base;
use plugin_base::{PluginRequest, PluginPeer};

pub use plugin_base::{Error, Spans, SpansBuilder};

const CHUNK_SIZE: usize = 1024 * 1024;

/// A handler that the plugin needs to instantiate.
pub trait Handler {
    fn init_buf(&mut self, ctx: PluginCtx, buf_size: usize);
    fn update(&mut self, ctx: PluginCtx);
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
}

pub struct PluginCtx<'a> {
    state: &'a mut State,
    peer: &'a PluginPeer,
}

pub fn mainloop<H: Handler>(handler: &mut H) {
    let mut state = State::default();
    plugin_base::mainloop(|req, peer| {
        let ctx = PluginCtx {
            state: &mut state,
            peer: peer,
        };
        match *req {
            PluginRequest::Ping => {
                print_err!("got ping");
                None
            }
            PluginRequest::InitBuf { buf_size, rev } => {
                print_err!("got init_buf buf_size = {}, rev = {}", buf_size, rev);
                ctx.state.buf_size = buf_size;
                ctx.state.rev = rev;
                handler.init_buf(ctx, buf_size);
                None
            }
            PluginRequest::Update { start, end, new_len, rev, edit_type } => {
                print_err!("got update notification {:?}", edit_type);
                ctx.state.buf_size = ctx.state.buf_size - (end - start) + new_len;
                ctx.state.rev = rev;
                // For now, invalidate everything.
                // TODO: use request params to actually update cache.
                ctx.state.cache = None;
                ctx.state.line_num = 0;
                ctx.state.offset_of_line = 0;
                handler.update(ctx);
                Some(Value::Null)
            }
        }
    });
}

impl<'a> PluginCtx<'a> {
    pub fn get_line(&mut self, line_num: usize) -> Result<Option<String>, Error> {
        if line_num != self.state.line_num {
            print_err!("can't handle non-sequential line numbers yet");
            return Ok(None);
        }
        if self.state.offset_of_line == self.state.buf_size {
            return Ok(None);
        }
        if self.state.cache.is_none() {
            self.state.cache = Some(try!(self.peer.get_data(self.state.offset_of_line, CHUNK_SIZE,
                self.state.rev)));
            self.state.cache_offset = self.state.offset_of_line;
        }
        loop {
            let offset_in_cache = self.state.offset_of_line - self.state.cache_offset;
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
                    let next_chunk = try!(self.peer.get_data(next_offset, CHUNK_SIZE,
                            self.state.rev));
                    self.state.cache_offset = self.state.offset_of_line;
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

    pub fn set_line_fg_spans(&self, line_num: usize, spans: Spans) {
        self.peer.set_line_fg_spans(line_num, spans)
    }
}

// TODO: use burntsushi memchr, or import this from xi_rope
fn memchr(needle: u8, haystack: &[u8]) -> Option<usize> {
    haystack.iter().position(|&b| b == needle)
}
