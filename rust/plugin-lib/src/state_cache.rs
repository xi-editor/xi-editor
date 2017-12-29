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
use serde_json::{self, Value};
use bytecount;
use rand::{thread_rng, Rng};

use xi_core::{PluginPid, ViewIdentifier, SyntaxDefinition, plugin_rpc};
use xi_rpc::{RemoteError, ReadError};

use plugin_base;
pub use plugin_base::Error;

type ConfigTable = serde_json::Map<String, Value>;

const CHUNK_SIZE: usize = 1024 * 1024;
const CACHE_SIZE: usize = 1024;

/// Number of probes for eviction logic.
const NUM_PROBES: usize = 5;

/// A handler that the plugin needs to instantiate.
pub trait Plugin {
    type State: Default + Clone;

    fn initialize(&mut self, ctx: PluginCtx<Self::State>, buf_size: usize);
    fn update(&mut self, ctx: PluginCtx<Self::State>, start: usize,
              end: usize, new_len: usize, rev: usize, text: &Option<String>)
              -> Option<Value>;
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
struct CacheState<S> {
    plugin_id: PluginPid,
    /// The length of the document in bytes.
    buf_size: usize,
    view_id: ViewIdentifier,
    rev: u64,

    /// A chunk of the document.
    chunk: String,
    /// Starting offset of the chunk.
    chunk_offset: usize,

    // cache of per-line state
    // Note: this doesn't store the 0 state, that's assumed
    state_cache: Vec<CacheEntry<S>>,

    /// The frontier, represented as a sorted list of line numbers.
    frontier: Vec<usize>,

    syntax: SyntaxDefinition,
    config: ConfigTable,
    path: Option<PathBuf>,
}

pub struct PluginCtx<'a, S: 'a> {
    state: &'a mut CacheState<S>,
    peer: plugin_base::PluginCtx<'a>,
}

struct CacheHandler<'a, P: Plugin + 'a> {
    handler: &'a mut P,
    state: CacheState<P::State>,
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
            Initialize { plugin_id, mut buffer_info } => {
                let info = buffer_info.remove(0);
                ctx.do_initialize(info, plugin_id, self.handler);
            }
            ConfigChanged { changes, .. } => ctx.do_config_changed(changes),
            DidSave { ref path, .. } => ctx.do_did_save(path, self.handler),
            NewBuffer { .. } | DidClose { .. } => eprintln!("Rust plugin lib \
            does not support global plugins"),
            //TODO: figure out shutdown
            Shutdown( .. ) => (),
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

pub fn mainloop<P: Plugin>(handler: &mut P) -> Result<(), ReadError>  {
    let mut my_handler = CacheHandler {
        handler: handler,
        state: CacheState::default(),
    };
    plugin_base::mainloop(&mut my_handler)
}

impl<'a, S: Default + Clone> PluginCtx<'a, S> {
    fn do_initialize<P>(mut self, init_info: plugin_rpc::PluginBufferInfo,
                        plugin_id: PluginPid, handler: &mut P)
        where P: Plugin<State = S>
    {
        let plugin_rpc::PluginBufferInfo {
            mut views, rev, buf_size,
            path, syntax, config, ..
        } = init_info;
        self.state.plugin_id = plugin_id;
        self.state.buf_size = buf_size;
        assert_eq!(views.len(), 1);
        self.state.view_id = views.remove(0);
        self.state.rev = rev;
        self.state.syntax = syntax;
        self.state.config = config;
        self.state.path = path.map(|p| PathBuf::from(p));
        self.truncate_frontier(0);
        handler.initialize(self, buf_size);
    }

    fn do_config_changed(self, changes: ConfigTable) {
        for (key, value) in changes.iter() {
            self.state.config.insert(key.to_owned(), value.to_owned());
        }
    }

    fn do_did_save<P: Plugin<State = S>>(self, path: &PathBuf, handler: &mut P) {
        self.state.path = Some(path.to_owned());
        handler.did_save(self);
    }

    fn do_update<P>(mut self, update: plugin_rpc::PluginUpdate, handler: &mut P) -> Value
        where P: Plugin<State = S>
    {
        let plugin_rpc::PluginUpdate { start, end, new_len, text, rev, .. } = update;
        self.state.buf_size = self.state.buf_size - (end - start) + new_len;
        self.state.rev = rev;
        if let Some(ref text) = text {
            let off = self.state.chunk_offset;
            if start >= off && start <= off + self.state.chunk.len() {
                let nlc_tail = if end <= off + self.state.chunk.len() {
                    let nlc = count_newlines(&self.state.chunk[start - off .. end - off]);
                    let tail = self.state.chunk[end - off ..].to_string();
                    Some((nlc, tail))
                } else {
                    None
                };
                self.state.chunk.truncate(start - off);
                self.state.chunk.push_str(&text);
                if let Some((newline_count, tail)) = nlc_tail {
                    self.state.chunk.push_str(&tail);
                    let new_nl_count = count_newlines(&text);
                    let nl_delta = new_nl_count.wrapping_sub(newline_count) as isize;
                    self.apply_delta(start, end, new_len, nl_delta);
                } else {
                    self.truncate_cache(start);
                }
            } else {
                self.state.chunk.clear();
                self.state.chunk_offset = 0;
                self.truncate_cache(start);
            }
        } else {
            self.state.chunk.clear();
            self.state.chunk_offset = 0;
            self.truncate_cache(start);
        }
        handler.update(self, start, end, new_len, rev as usize, &text)
            .unwrap_or(Value::from(0i32))
    }

    pub fn get_path(&self) -> Option<&PathBuf> {
        match self.state.path {
            Some(ref p) => Some(p),
            None => None,
        }
    }

    pub fn get_config(&self) -> &ConfigTable {
        &self.state.config
    }

    pub fn get_syntax(&self) -> SyntaxDefinition {
        self.state.syntax
    }

    pub fn add_scopes(&self, scopes: &Vec<Vec<String>>) {
        self.peer.add_scopes(self.state.plugin_id, &self.state.view_id, scopes)
    }

    pub fn update_spans(&self, start: usize, len: usize,
                        spans: &[plugin_rpc::ScopeSpan]) {
        self.peer.update_spans(self.state.plugin_id, &self.state.view_id,
                               start, len, self.state.rev, spans)
    }

    pub fn edit(&self, plugin_id: PluginPid, start: usize, end: usize, rev: usize,
                after_cursor: bool, text: &str, author: &str, priority: usize) {
        self.peer.edit(self.state.plugin_id, start, end, rev, after_cursor,
                       text, author, priority);
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

    /// Find an entry in the cache by offset. Similar to `find_line`.
    pub fn find_offset(&self, offset: usize) -> Result<usize, usize> {
        self.state.state_cache.binary_search_by(|probe| probe.offset.cmp(&offset))
    }

    /// Get the state from the nearest cache entry at or before given line number.
    /// Returns line number, offset, and user state.
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

    /// Get the state at the given line number, if it exists in the cache.
    pub fn get(&self, line_num: usize) -> Option<&S> {
        self.find_line(line_num).ok()
            .and_then(|ix| self.state.state_cache[ix].user_state.as_ref())
    }

    /// Set the state at the given line number. Note: has no effect if line_num
    /// references the end of the partial line at EOF.
    pub fn set(&mut self, line_num: usize, s: S) {
        if let Some(entry) = self.get_entry(line_num) {
            entry.user_state = Some(s);
        }
    }

    /// Get the cache entry at the given line number, creating it if necessary.
    /// Returns None if line_num > number of newlines in doc (ie if it references
    /// the end of the partial line at EOF).
    fn get_entry(&mut self, line_num: usize) -> Option<&mut CacheEntry<S>> {
        match self.find_line(line_num) {
            Ok(ix) => Some(&mut self.state.state_cache[ix]),
            Err(_ix) => {
                // TODO: could get rid of redundant binary search
                let (offset, _ix, partial) = self.get_offset_ix_of_line(line_num)
                    .expect("TODO return result");
                if partial {
                    None
                } else {
                    let new_ix = self.insert_entry(line_num, offset, None);
                    Some(&mut self.state.state_cache[new_ix])
                }
            }
        }
    }

    /// Insert a new entry into the cache, returning its index.
    fn insert_entry(&mut self, line_num: usize, offset: usize, user_state: Option<S>) -> usize {
        if self.state.state_cache.len() >= CACHE_SIZE {
            self.evict();
        }
        match self.find_line(line_num) {
            Ok(_ix) => panic!("entry already exists"),
            Err(ix) => {
                self.state.state_cache.insert(ix, CacheEntry {
                    line_num, offset, user_state
                });
                ix
            }
        }
    }

    /// Evict one cache entry.
    fn evict(&mut self) {
        let ix = self.choose_victim();
        self.state.state_cache.remove(ix);
    }

    fn choose_victim(&self) -> usize {
        let mut best = None;
        let mut rng = thread_rng();
        for _ in 0..NUM_PROBES {
            let ix = rng.gen_range(0, self.state.state_cache.len());
            let gap = self.compute_gap(ix);
            if best.map(|(last_gap, _)| gap < last_gap).unwrap_or(true) {
                best = Some((gap, ix));
            }
        }
        best.unwrap().1
    }

    /// Compute the gap that would result after deleting the given entry.
    fn compute_gap(&self, ix: usize) -> usize {
        let before = if ix == 0 { 0 } else { self.state.state_cache[ix - 1].offset };
        let after = if let Some(item) = self.state.state_cache.get(ix + 1) {
            item.offset
        } else {
            self.state.buf_size
        };
        after - before
    }

    fn fetch_chunk(&mut self, start: usize) -> Result<String, Error> {
        self.peer.get_data(self.state.plugin_id, &self.state.view_id,
                           start, CHUNK_SIZE, self.state.rev)
    }

    /// Get a slice of the document, containing at least the given interval
    /// plus at least one more codepoint (unless at EOF).
    fn get_chunk(&mut self, start: usize, end: usize) -> Result<&str, Error> {
        loop {
            let chunk_start = self.state.chunk_offset;
            let chunk_end = chunk_start + self.state.chunk.len();
            if start >= chunk_start && (start < chunk_end || chunk_end == self.state.buf_size) {
                // At least the first codepoint at start is in the chunk.
                if end < chunk_end || chunk_end == self.state.buf_size {
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

    /// Returns the offset, the index in the cache, and a bool indicating whether it's a
    /// partial line at EOF.
    fn get_offset_ix_of_line(&mut self, line_num: usize) -> Result<(usize, usize, bool), Error> {
        if line_num == 0 {
            return Ok((0, 0, false));
        }
        match self.find_line(line_num) {
            Ok(ix) => Ok((self.state.state_cache[ix].offset, ix, false)),
            Err(ix) => {
                let (mut l, mut offset) = if ix == 0 { (0, 0) } else {
                    let item = &self.state.state_cache[ix - 1];
                    (item.line_num, item.offset)
                };
                let mut end = offset;
                loop {
                    if end == self.state.buf_size {
                        return Ok((end, ix, true));
                    }
                    let chunk = self.get_chunk(offset, end)?;
                    if let Some(pos) = memchr(b'\n', chunk.as_bytes()) {
                        offset += pos + 1;
                        l += 1;
                        if l == line_num {
                            return Ok((offset, ix, false));
                        }
                    } else {
                        end = offset + chunk.len();
                    }
                }
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

    /// Get the line at the given line. Returns empty string if at EOF.
    pub fn get_line(&mut self, line_num: usize) -> Result<&str, Error> {
        let (start, _ix, _partial) = self.get_offset_ix_of_line(line_num)?;
        // TODO: if cache entry at ix + 1 has line_num + 1, then we know line len
        let len = self.get_line_len(start)?;
        // TODO: should store offset of next line, to avoid re-scanning

        // TODO: this will pull in the first codepoint of the next line, which
        // is not necessary.
        let chunk = self.get_chunk(start, start + len)?;
        Ok(&chunk[..len])
    }

    /// Release all state _after_ the given offset.
    fn truncate_cache(&mut self, offset: usize) {
        let (line_num, ix) = match self.find_offset(offset) {
            Ok(ix) => (self.state.state_cache[ix].line_num, ix + 1),
            Err(ix) => (
                if ix == 0 { 0 } else {
                    self.state.state_cache[ix - 1].line_num
                },
                ix
            ),
        };
        self.truncate_frontier(line_num);
        self.state.state_cache.truncate(ix);
    }

    fn truncate_frontier(&mut self, line_num: usize) {
        match self.state.frontier.binary_search(&line_num) {
            Ok(ix) => self.state.frontier.truncate(ix + 1),
            Err(ix) => {
                self.state.frontier.truncate(ix);
                self.state.frontier.push(line_num);
            }
        }
    }

    /// The contents between `start` and `end` have been replaced with
    /// new content of size `new_bytes`. Clears all state in the interior
    /// of that region, fixes up line and offset info, and sets a frontier
    /// at the beginning of the region.
    fn apply_delta(&mut self, start: usize, end: usize, new_bytes: usize,
        nl_count_delta: isize)
    {
        let ix = match self.find_offset(start) {
            Ok(ix) => ix + 1,
            Err(ix) => ix,
        };
        // Note: the "<=" can be tightened to "<" in some circumstances, but the logic
        // is complicated.
        while ix < self.state.state_cache.len() && self.state.state_cache[ix].offset <= end {
            self.state.state_cache.remove(ix);
        }
        let off_delta = (start + new_bytes).wrapping_sub(end);
        if off_delta != 0 || nl_count_delta != 0 {
            for entry in &mut self.state.state_cache[ix..] {
                entry.line_num = entry.line_num.wrapping_add(nl_count_delta as usize);
                entry.offset = entry.offset.wrapping_add(off_delta);
            }
        }
        let line_num = if ix == 0 { 0 } else { self.state.state_cache[ix - 1].line_num };
        let mut new_frontier = Vec::new();
        let mut need_push = true;
        for old_ln in &self.state.frontier {
            if *old_ln < line_num {
                new_frontier.push(*old_ln);
            } else {
                if need_push {
                    new_frontier.push(line_num);
                    need_push = false;
                    if let Some(ref entry) = self.state.state_cache.get(ix) {
                        if *old_ln >= entry.line_num {
                            new_frontier.push(old_ln.wrapping_add(nl_count_delta as usize));
                        }
                    }
                }
            }
        }
        if need_push {
            new_frontier.push(line_num);
        }
        self.state.frontier = new_frontier;
    }

    /// Clear all state and reset frontier to start.
    pub fn reset(&mut self) {
        self.truncate_cache(0);
    }

    /// The frontier keeps track of work needing to be done. A typical
    /// user will call `get_frontier` to get a line number, do the work
    /// on that line, insert state for the next line, and then call either
    /// `update_frontier` or `close_frontier` depending on whether there
    /// is more work to be done at that location.
    pub fn get_frontier(&self) -> Option<usize> {
        self.state.frontier.first().cloned()
    }

    /// Updates the frontier. This can go backward, but most typically
    /// goes forward by 1 line (compared to the `get_frontier` result).
    pub fn update_frontier(&mut self, new_frontier: usize) {
        if self.state.frontier.get(1) == Some(&new_frontier) {
            self.state.frontier.remove(0);
        } else {
            self.state.frontier[0] = new_frontier;
        }
    }

    /// Closes the current frontier. This is the correct choice to handle
    /// EOF.
    pub fn close_frontier(&mut self) {
        self.state.frontier.remove(0);
    }
}

// TODO: use burntsushi memchr, or import this from xi_rope
fn memchr(needle: u8, haystack: &[u8]) -> Option<usize> {
    haystack.iter().position(|&b| b == needle)
}

fn count_newlines(s: &str) -> usize {
    bytecount::count(s.as_bytes(), b'\n')
}
