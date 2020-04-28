// Copyright 2016 The xi-editor Authors.
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

use rand::{thread_rng, Rng};

use xi_rope::interval::IntervalBounds;
use xi_rope::{LinesMetric, RopeDelta};
use xi_trace::trace_block;

use super::{Cache, DataSource, Error, View};
use crate::base_cache::ChunkCache;

const CACHE_SIZE: usize = 1024;

/// Number of probes for eviction logic.
const NUM_PROBES: usize = 5;

struct CacheEntry<S> {
    line_num: usize,
    offset: usize,
    user_state: Option<S>,
}

/// The caching state
#[derive(Default)]
pub struct StateCache<S> {
    pub(crate) buf_cache: ChunkCache,
    state_cache: Vec<CacheEntry<S>>,
    /// The frontier, represented as a sorted list of line numbers.
    frontier: Vec<usize>,
}

impl<S: Clone + Default> Cache for StateCache<S> {
    fn new(buf_size: usize, rev: u64, num_lines: usize) -> Self {
        StateCache {
            buf_cache: ChunkCache::new(buf_size, rev, num_lines),
            state_cache: Vec::new(),
            frontier: Vec::new(),
        }
    }

    fn get_line<DS: DataSource>(&mut self, source: &DS, line_num: usize) -> Result<&str, Error> {
        self.buf_cache.get_line(source, line_num)
    }

    fn get_region<DS, I>(&mut self, source: &DS, interval: I) -> Result<&str, Error>
    where
        DS: DataSource,
        I: IntervalBounds,
    {
        self.buf_cache.get_region(source, interval)
    }

    fn get_document<DS: DataSource>(&mut self, source: &DS) -> Result<String, Error> {
        self.buf_cache.get_document(source)
    }

    fn offset_of_line<DS: DataSource>(
        &mut self,
        source: &DS,
        line_num: usize,
    ) -> Result<usize, Error> {
        self.buf_cache.offset_of_line(source, line_num)
    }

    fn line_of_offset<DS: DataSource>(
        &mut self,
        source: &DS,
        offset: usize,
    ) -> Result<usize, Error> {
        self.buf_cache.line_of_offset(source, offset)
    }

    /// Updates the cache by applying this delta.
    fn update(&mut self, delta: Option<&RopeDelta>, buf_size: usize, num_lines: usize, rev: u64) {
        let _t = trace_block("StateCache::update", &["plugin"]);

        if let Some(ref delta) = delta {
            self.update_line_cache(delta);
        } else {
            // if there's no delta (very large edit) we blow away everything
            self.clear_to_start(0);
        }

        self.buf_cache.update(delta, buf_size, num_lines, rev);
    }

    /// Flushes any state held by this cache.
    fn clear(&mut self) {
        self.reset()
    }
}

impl<S: Clone + Default> StateCache<S> {
    /// Find an entry in the cache by line num. On return `Ok(i)` means entry
    /// at index `i` is an exact match, while `Err(i)` means the entry would be
    /// inserted at `i`.
    fn find_line(&self, line_num: usize) -> Result<usize, usize> {
        self.state_cache.binary_search_by(|probe| probe.line_num.cmp(&line_num))
    }

    /// Find an entry in the cache by offset. Similar to `find_line`.
    pub fn find_offset(&self, offset: usize) -> Result<usize, usize> {
        self.state_cache.binary_search_by(|probe| probe.offset.cmp(&offset))
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
                let item = &self.state_cache[ix];
                if let Some(ref s) = item.user_state {
                    return (item.line_num, item.offset, s.clone());
                }
                if ix == 0 {
                    break;
                }
                ix -= 1;
            }
        }
        (0, 0, S::default())
    }

    /// Get the state at the given line number, if it exists in the cache.
    pub fn get(&self, line_num: usize) -> Option<&S> {
        self.find_line(line_num).ok().and_then(|ix| self.state_cache[ix].user_state.as_ref())
    }

    /// Set the state at the given line number. Note: has no effect if line_num
    /// references the end of the partial line at EOF.
    pub fn set<DS>(&mut self, source: &DS, line_num: usize, s: S)
    where
        DS: DataSource,
    {
        if let Some(entry) = self.get_entry(source, line_num) {
            entry.user_state = Some(s);
        }
    }

    /// Get the cache entry at the given line number, creating it if necessary.
    /// Returns None if line_num > number of newlines in doc (ie if it references
    /// the end of the partial line at EOF).
    fn get_entry<DS>(&mut self, source: &DS, line_num: usize) -> Option<&mut CacheEntry<S>>
    where
        DS: DataSource,
    {
        match self.find_line(line_num) {
            Ok(ix) => Some(&mut self.state_cache[ix]),
            Err(_ix) => {
                if line_num == self.buf_cache.num_lines {
                    None
                } else {
                    let offset = self
                        .buf_cache
                        .offset_of_line(source, line_num)
                        .expect("get_entry should validate inputs");
                    let new_ix = self.insert_entry(line_num, offset, None);
                    Some(&mut self.state_cache[new_ix])
                }
            }
        }
    }

    /// Insert a new entry into the cache, returning its index.
    fn insert_entry(&mut self, line_num: usize, offset: usize, user_state: Option<S>) -> usize {
        if self.state_cache.len() >= CACHE_SIZE {
            self.evict();
        }
        match self.find_line(line_num) {
            Ok(_ix) => panic!("entry already exists"),
            Err(ix) => {
                self.state_cache.insert(ix, CacheEntry { line_num, offset, user_state });
                ix
            }
        }
    }

    /// Evict one cache entry.
    fn evict(&mut self) {
        let ix = self.choose_victim();
        self.state_cache.remove(ix);
    }

    fn choose_victim(&self) -> usize {
        let mut best = None;
        let mut rng = thread_rng();
        for _ in 0..NUM_PROBES {
            let ix = rng.gen_range(0, self.state_cache.len());
            let gap = self.compute_gap(ix);
            if best.map(|(last_gap, _)| gap < last_gap).unwrap_or(true) {
                best = Some((gap, ix));
            }
        }
        best.unwrap().1
    }

    /// Compute the gap that would result after deleting the given entry.
    fn compute_gap(&self, ix: usize) -> usize {
        let before = if ix == 0 { 0 } else { self.state_cache[ix - 1].offset };
        let after = if let Some(item) = self.state_cache.get(ix + 1) {
            item.offset
        } else {
            self.buf_cache.buf_size
        };
        assert!(after >= before, "{} < {} ix: {}", after, before, ix);
        after - before
    }

    /// Release all state _after_ the given offset.
    fn truncate_cache(&mut self, offset: usize) {
        let (line_num, ix) = match self.find_offset(offset) {
            Ok(ix) => (self.state_cache[ix].line_num, ix + 1),
            Err(ix) => (if ix == 0 { 0 } else { self.state_cache[ix - 1].line_num }, ix),
        };
        self.truncate_frontier(line_num);
        self.state_cache.truncate(ix);
    }

    pub(crate) fn truncate_frontier(&mut self, line_num: usize) {
        match self.frontier.binary_search(&line_num) {
            Ok(ix) => self.frontier.truncate(ix + 1),
            Err(ix) => {
                self.frontier.truncate(ix);
                self.frontier.push(line_num);
            }
        }
    }

    /// Updates the line cache to reflect this delta.
    fn update_line_cache(&mut self, delta: &RopeDelta) {
        let (iv, new_len) = delta.summary();
        if let Some(n) = delta.as_simple_insert() {
            assert_eq!(iv.size(), 0);
            assert_eq!(new_len, n.len());

            let newline_count = n.measure::<LinesMetric>();
            self.line_cache_simple_insert(iv.start(), new_len, newline_count);
        } else if delta.is_simple_delete() {
            assert_eq!(new_len, 0);
            self.line_cache_simple_delete(iv.start(), iv.end())
        } else {
            self.clear_to_start(iv.start());
        }
    }

    fn line_cache_simple_insert(&mut self, start: usize, new_len: usize, newline_num: usize) {
        let ix = match self.find_offset(start) {
            Ok(ix) => ix + 1,
            Err(ix) => ix,
        };

        for entry in &mut self.state_cache[ix..] {
            entry.line_num += newline_num;
            entry.offset += new_len;
        }
        self.patchup_frontier(ix, newline_num as isize);
    }

    fn line_cache_simple_delete(&mut self, start: usize, end: usize) {
        let off = self.buf_cache.offset;
        let chunk_end = off + self.buf_cache.contents.len();
        if start >= off && end <= chunk_end {
            let del_newline_num = count_newlines(&self.buf_cache.contents[start - off..end - off]);
            // delete all entries that overlap the deleted range
            let ix = match self.find_offset(start) {
                Ok(ix) => ix + 1,
                Err(ix) => ix,
            };
            while ix < self.state_cache.len() && self.state_cache[ix].offset <= end {
                self.state_cache.remove(ix);
            }
            for entry in &mut self.state_cache[ix..] {
                entry.line_num -= del_newline_num;
                entry.offset -= end - start;
            }
            self.patchup_frontier(ix, -(del_newline_num as isize));
        } else {
            // if this region isn't in our chunk we can't correctly adjust newlines
            self.clear_to_start(start);
        }
    }

    fn patchup_frontier(&mut self, cache_idx: usize, nl_count_delta: isize) {
        let line_num = match cache_idx {
            0 => 0,
            ix => self.state_cache[ix - 1].line_num,
        };
        let mut new_frontier = Vec::new();
        let mut need_push = true;
        for old_ln in &self.frontier {
            if *old_ln < line_num {
                new_frontier.push(*old_ln);
            } else if need_push {
                new_frontier.push(line_num);
                need_push = false;
                if let Some(ref entry) = self.state_cache.get(cache_idx) {
                    if *old_ln >= entry.line_num {
                        new_frontier.push(old_ln.wrapping_add(nl_count_delta as usize));
                    }
                }
            }
        }
        if need_push {
            new_frontier.push(line_num);
        }
        self.frontier = new_frontier;
    }

    /// Clears any cached text and anything in the state cache before `start`.
    fn clear_to_start(&mut self, start: usize) {
        self.truncate_cache(start);
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
        self.frontier.first().cloned()
    }

    /// Updates the frontier. This can go backward, but most typically
    /// goes forward by 1 line (compared to the `get_frontier` result).
    pub fn update_frontier(&mut self, new_frontier: usize) {
        if self.frontier.get(1) == Some(&new_frontier) {
            self.frontier.remove(0);
        } else {
            self.frontier[0] = new_frontier;
        }
    }

    /// Closes the current frontier. This is the correct choice to handle
    /// EOF.
    pub fn close_frontier(&mut self) {
        self.frontier.remove(0);
    }
}

/// StateCache specific extensions on `View`
impl<S: Default + Clone> View<StateCache<S>> {
    pub fn get_frontier(&self) -> Option<usize> {
        self.cache.get_frontier()
    }

    pub fn get_prev(&self, line_num: usize) -> (usize, usize, S) {
        self.cache.get_prev(line_num)
    }

    pub fn get(&self, line_num: usize) -> Option<&S> {
        self.cache.get(line_num)
    }

    pub fn set(&mut self, line_num: usize, s: S) {
        let ctx = self.make_ctx();
        self.cache.set(&ctx, line_num, s)
    }

    pub fn update_frontier(&mut self, new_frontier: usize) {
        self.cache.update_frontier(new_frontier)
    }

    pub fn close_frontier(&mut self) {
        self.cache.close_frontier()
    }

    pub fn reset(&mut self) {
        self.cache.reset()
    }

    pub fn find_offset(&self, offset: usize) -> Result<usize, usize> {
        self.cache.find_offset(offset)
    }
}

fn count_newlines(s: &str) -> usize {
    bytecount::count(s.as_bytes(), b'\n')
}
