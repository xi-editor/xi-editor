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

//! A more sophisticated cache that manages user state.

use std::path::PathBuf;
use serde_json::Value;

use plugin_base;
use plugin_base::PluginRequest;

pub use plugin_base::{Error, ScopeSpan};

const CHUNK_SIZE: usize = 1024 * 1024;

/// A handler that the plugin needs to instantiate.
pub trait Handler {
    type State: Default + Clone;

    fn initialize(&mut self, ctx: PluginCtx<Self::State>, buf_size: usize);
    fn update(&mut self, ctx: PluginCtx<Self::State>);
    fn did_save(&mut self, ctx: PluginCtx<Self::State>);
    #[allow(unused_variables)]
    fn idle(&mut self, ctx: PluginCtx<Self::State>, token: usize) {}
}

struct CacheEntry<S> {
    line_num: usize,
    offset: usize,
    user_state: Option<S>,
}

/// The caching state
#[derive(Default)]
struct MyState<S> {
    /// The length of the document in bytes.
    buf_size: usize,
    view_id: String,
    rev: u64,
    cache: Option<String>,
    cache_offset: usize,

    // line iteration state
    line_num: usize,
    offset_of_line: usize,

    /// A chunk of the document.
    chunk: String,
    /// Starting offset of the chunk.
    chunk_offset: usize,

    // cache of per-line state
    // Note: this doesn't store the 0 state, that's assumed
    state_cache: Vec<CacheEntry<S>>,

    syntax: String,
    path: Option<PathBuf>,
}

pub struct PluginCtx<'a, S: 'a> {
    state: &'a mut MyState<S>,
    peer: plugin_base::PluginCtx<'a>,
}

struct MyHandler<'a, H: Handler + 'a> {
    handler: &'a mut H,
    state: MyState<H::State>,
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
                assert_eq!(init_info.views.len(), 1);
                ctx.state.view_id = init_info.views[0].clone();
                ctx.state.rev = init_info.rev;
                ctx.state.syntax = init_info.syntax.clone();
                ctx.state.path = init_info.path.clone().map(|p| PathBuf::from(p));
                self.handler.initialize(ctx, init_info.buf_size);
                None
            }
            PluginRequest::Update { start, end, new_len, rev, text, .. } => {
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
            PluginRequest::DidSave { ref path } => {
                let new_path = Some(path.to_owned());
                if ctx.state.path != new_path {
                    ctx.state.line_num = 0;
                    ctx.state.offset_of_line = 0;
                }
                ctx.state.path = Some(path.to_owned());
                self.handler.did_save(ctx);
                None
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
        state: MyState::default(),
    };
    plugin_base::mainloop(&mut my_handler);
}

impl<'a, S: Default + Clone> PluginCtx<'a, S> {

    pub fn get_path(&self) -> Option<&PathBuf> {
        match self.state.path {
            Some(ref p) => Some(p),
            None => None,
        }
    }

    pub fn add_scopes(&self, scopes: &Vec<Vec<String>>) {
        self.peer.add_scopes(&self.state.view_id, scopes)
    }

    pub fn update_spans(&self, start: usize, len: usize, spans: &[ScopeSpan]) {
        self.peer.update_spans(&self.state.view_id, start, len, self.state.rev, spans)
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

    /// Find an entry in the cache by line num. On return `Ok(i)` means entry
    /// at index `i` is an exact match, while `Err(i)` means the entry would be
    /// inserted at `i`.
    fn find_line(&self, line_num: usize) -> Result<usize, usize> {
        self.state.state_cache.binary_search_by(|probe| probe.line_num.cmp(&line_num))
    }

    /// Get state at or before given line number.
    pub fn get_prev(&self, line_num: usize) -> (usize, usize, S) {
        if line_num > 0 {
            let mut ix = match self.find_line(line_num) {
                Ok(ix) => ix,
                Err(0) => return (0, 0, S::default()),
                Err(ix) => ix - 1,
            };
            loop {
                let item = &self.state.state_cache[ix];
                if let Some(ref s) = item.user_state {
                    return (item.line_num, item.offset, s.clone());
                }
                if ix == 0 { break; }
                ix -= 1;
            }
        }
        (0, 0, S::default())
    }

    /// Set the state at the given line number.
    pub fn set(&mut self, line_num: usize, s: S) {
        self.get_entry(line_num).user_state = Some(s);
    }

    /// Get the cache entry at the given line number, creating it if necessary.
    fn get_entry(&mut self, line_num: usize) -> &mut CacheEntry<S> {
        match self.find_line(line_num) {
            Ok(ix) => &mut self.state.state_cache[ix],
            Err(ix) => unimplemented!(),
        }
    }

    /// Insert a new entry into the cache, returning its index.
    fn insert_entry(&mut self, line_num: usize, offset: usize, user_state: Option<S>) -> usize {
        match self.find_line(line_num) {
            // TODO: evict if full
            Ok(ix) => panic!("entry already exists"),
            Err(ix) => {
                self.state.state_cache.insert(ix, CacheEntry {
                    line_num, offset, user_state
                });
                ix
            }
        }
    }

    fn fetch_chunk(&mut self, start: usize) -> Result<String, Error> {
        self.peer.get_data(&self.state.view_id, start, CHUNK_SIZE, self.state.rev)
    }

    /// Get a slice of the document, containing at least the given interval
    /// plus at least one more codepoint (unless at EOF).
    fn get_chunk(&mut self, start: usize, end: usize) -> Result<&str, Error> {
        loop {
            let chunk_start = self.state.chunk_offset;
            let chunk_end = chunk_start + self.state.chunk.len();
            if start >= chunk_start && start < chunk_end {
                // At least the first codepoint at start is in the chunk
                if end < chunk_end || end == self.state.buf_size {
                    return Ok(&self.state.chunk[start - chunk_start ..]);
                }
                let new_chunk = self.fetch_chunk(chunk_end)?;
                if start == chunk_start {
                    self.state.chunk.push_str(&new_chunk);
                } else {
                    self.state.chunk_offset = start;
                    self.state.chunk = [&self.state.chunk[start - chunk_start ..],
                        &new_chunk].concat();
                }
            } else {
                // TODO: if chunk_start < start + CHUNK_SIZE, could fetch smaller
                // chunk and concat; probably not a major savings in practice.
                self.state.chunk = self.fetch_chunk(start)?;
                self.state.chunk_offset = start;
            }
        }
    }

    fn get_offset_ix_of_line(&mut self, line_num: usize) -> Result<(usize, usize), Error> {
        if line_num == 0 {
            return Ok((0, 0));
        }
        match self.find_line(line_num) {
            Ok(ix) => Ok((self.state.state_cache[ix].offset, ix)),
            Err(ix) => {
                let (mut l, mut offset) = if ix == 0 { (0, 0) } else {
                    let item = &self.state.state_cache[ix - 1];
                    (item.line_num, item.offset)
                };
                let mut end = offset;
                loop {
                    let chunk = self.get_chunk(offset, end)?;
                    if let Some(pos) = memchr(b'\n', chunk.as_bytes()) {
                        offset += pos + 1;
                        l += 1;
                        if l == line_num {
                            return Ok((offset, ix));
                        }
                    } else {
                        end = offset + chunk.len();
                    }
                }
                unimplemented!()
            }
        }
    }

    fn get_line_len(&mut self, start: usize) -> Result<usize, Error> {
        let mut end = start;
        loop {
            let buf_size = self.state.buf_size;
            let chunk = self.get_chunk(start, end)?;
            match memchr(b'\n', chunk.as_bytes()) {
                Some(pos) => return Ok(pos + 1),
                None => {
                    end = start + chunk.len();
                    if end == buf_size {
                        return Ok(chunk.len());
                    }
                }
            }
        }
    }

    pub fn get_line(&mut self, line_num: usize) -> Result<&str, Error> {
        let (start, _ix) = self.get_offset_ix_of_line(line_num)?;
        // TODO: if cache entry at ix + 1 has line_num + 1, then we know line len
        let len = self.get_line_len(start)?;
        // TODO: this will pull in the first codepoint of the next line, which
        // is not necessary.
        let chunk = self.get_chunk(start, start + len)?;
        Ok(&chunk[start .. start + len])
    }

}

// TODO: use burntsushi memchr, or import this from xi_rope
fn memchr(needle: u8, haystack: &[u8]) -> Option<usize> {
    haystack.iter().position(|&b| b == needle)
}
