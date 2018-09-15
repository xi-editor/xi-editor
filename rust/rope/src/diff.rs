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

//! Computing deltas between two ropes.

use std::collections::HashMap;

use compare::RopeScanner;
use delta::{Delta, DeltaElement};
use interval::Interval;
use rope::{Rope, RopeInfo, RopeDelta};
use tree::{Node, NodeInfo};

use memchr::memchr;

/// A trait implemented by various diffing strategies.
pub trait Diff<N: NodeInfo> {
    fn compute_delta(base: &Node<N>, target: &Node<N>) -> Delta<N>;
}

/// The minimum length of non-whitespace characters in a line before
/// we consider it for diffing purposes.
const MIN_SIZE: usize = 32;

/// A line-oriented, hash based diff algorithm.
///
/// This works by taking a hash of each line in either document that
/// has a length, ignoring leading whitespace, above some threshold.
///
/// Lines in the target document are matched against lines in the
/// base document. When a match is found, it is extended forwards
/// and backwards as far as possible.
///
/// This runs in O(n+m) in the lengths of the two ropes, and produces
/// results on a variety of workloads that are comparable in quality
/// (measured in terms of serialized diff size) with the results from
/// using a suffix array, while being an order of magnitude faster.
pub struct LineHashDiff;

impl Diff<RopeInfo> for LineHashDiff {
    fn compute_delta(base: &Rope, target: &Rope) -> RopeDelta {
        let mut builder = DiffBuilder::default();

        // before doing anything, scan top down and bottom up for like-ness.
        let mut scanner = RopeScanner::new(base, target);
        let (start_offset, diff_end) = scanner.find_min_diff_range();
        let target_end = target.len() - diff_end;

        if start_offset > 0 {
            builder.copy(0, 0, start_offset);
        }

        // if our preliminary scan finds no differences we're done
        if start_offset == base.len() && target.len() == base.len() {
            return builder.to_delta(base, target);
        }

        //TODO: because of how `lines_raw` returns Cows, we can't easily build
        //the lookup table without allocating. The eventual solution would be to have
        //a custom iter on the rope that returns suitable chunks.
        let base_string = String::from(base);
        let line_hashes = make_line_hashes(&base_string, MIN_SIZE);

        let mut offset = start_offset;

        // When we find a matching region, we extend it forwards and backwards.
        // we keep track of how far forward we extend it each time, to avoid
        // having a subsequent scan extend backwards over the same region.
        let mut prev_targ_end = start_offset;
        let mut prev_base_end = 0;

        for line in target.lines_raw(start_offset..target_end) {
            let non_ws = non_ws_offset(&line);
            if offset + non_ws < prev_targ_end {
                // no-op, but we don't break because we still want to bump offset
            } else if line.len() - non_ws >= MIN_SIZE {
                if let Some(base_off) = line_hashes.get(&line[non_ws..]) {
                    let targ_off = offset + non_ws;
                    let (left_dist, mut right_dist) = fast_expand_match(base, target,
                                                                        *base_off,
                                                                        targ_off,
                                                                        prev_targ_end);
                    if targ_off + right_dist > target_end {
                        // don't let last match expand past target_end
                        right_dist = target_end - targ_off;
                    }
                    let targ_start = targ_off - left_dist;
                    let base_start = base_off - left_dist;
                    let len = left_dist + right_dist;

                    // other parts of the code (Delta::factor) require that delta ops
                    // be in non-decreasing order, so we only actually copy a region
                    // when this is true. This algorithm was initially designed without
                    // this constraint; a better design would prioritize early matches,
                    // and more efficiently avoid searching in disallowed regions.
                    if base_start >= prev_base_end {
                        builder.copy(base_start, targ_start, len);
                        prev_targ_end = targ_start + len;
                        prev_base_end = base_start + len;
                    }
                }
            }
            offset += line.len();
        }

        if diff_end > 0 {
            builder.copy(base.len() - diff_end, target.len() - diff_end, diff_end);
        }

        builder.to_delta(base, target)
    }
}

/// Given two ropes and the offsets of two equal bytes, finds the largest
/// identical substring shared between the two ropes which contains the offset.
///
/// The return value is a pair of offsets, each of which represents an absolute
/// distance. That is to say, the position of the start and end boundaries
/// relative to the input offset.
fn fast_expand_match(base: &Rope, target: &Rope, base_off: usize, targ_off: usize,
                prev_match_targ_end: usize) -> (usize, usize) {

    let mut scanner = RopeScanner::new(base, target);
    let max_left = targ_off - prev_match_targ_end;
    let start = scanner.find_ne_char_left(base_off, targ_off, max_left);
    debug_assert!(start <= max_left, "{} <= {}", start, max_left);
    let end = scanner.find_ne_char_right(base_off, targ_off, None);
    (start.min(max_left), end)
}

#[inline]
fn non_ws_offset(s: &str) -> usize {
    s.as_bytes()
        .iter()
        .take_while(|b| **b == b' ' || **b == b'\t')
        .count()
}

/// Represents copying `len` bytes from base to target.
#[derive(Debug, Clone, Copy)]
struct DiffOp {
    target_idx: usize,
    base_idx: usize,
    len: usize,
}

/// Keeps track of copy ops during diff construction.
#[derive(Debug, Clone, Default)]
pub struct DiffBuilder {
    ops: Vec<DiffOp>,
}

impl DiffBuilder {
    fn copy(&mut self, base: usize, target: usize, len: usize) {
        if let Some(prev) = self.ops.last_mut() {
            let prev_end = prev.target_idx + prev.len;
            let base_end = prev.base_idx + prev.len;
            assert!(prev_end <= target, "{} <= {} prev {:?}", prev_end, target, prev);
            if prev_end == target && base_end == base {
                prev.len += len;
                return;
            }
        }
        self.ops.push(DiffOp { target_idx: target, base_idx: base, len: len })
    }

    fn to_delta(self, base: &Rope, target: &Rope) -> RopeDelta {
        let mut els = Vec::with_capacity(self.ops.len() * 2);
        let mut targ_pos = 0;
        for DiffOp { base_idx, target_idx, len } in self.ops {
            if target_idx > targ_pos {
                let iv = Interval::new_closed_open(targ_pos, target_idx);
                els.push(DeltaElement::Insert(target.subseq(iv)));
            }
            els.push(DeltaElement::Copy(base_idx, base_idx + len));
            targ_pos = target_idx + len;
        }

        if targ_pos < target.len() {
            let iv = Interval::new_closed_open(targ_pos, target.len());
            els.push(DeltaElement::Insert(target.subseq(iv)));
        }

        Delta { els, base_len: base.len() }
    }
}

/// Fast iterator over lines in a string, not removing newline characters.
struct LineScanner<'a> {
    inner: &'a str,
    idx: usize,
}

impl<'a> Iterator for LineScanner<'a> {
    type Item = &'a str;
    fn next(&mut self) -> Option<&'a str> {
        let idx = memchr(b'\n', &self.inner.as_bytes()[self.idx..])?;
        let result = &self.inner[self.idx..self.idx + idx+1];
        self.idx += idx + 1;
        Some(result)
    }
}

fn make_line_hashes<'a>(base: &'a str, min_size: usize) -> HashMap<&'a str, usize> {
    let mut offset = 0;
    let mut line_hashes = HashMap::with_capacity(base.len() / 60);
    let iter = LineScanner { inner: base, idx: 0 };
    for line in iter {
        let non_ws = non_ws_offset(&line);
        if line.len() - non_ws >= min_size {
            line_hashes.insert(&line[non_ws..], offset + non_ws);
        }
        offset += line.len();
    }
    line_hashes
}


#[cfg(test)]
mod tests {
    use super::*;

    static SMALL_ONE: &str = "This adds FixedSizeAdler32, that has a size set at construction, and keeps bytes in a cyclic buffer of that size to be removed when it fills up.

Current logic (and implementing Write) might be too much, since bytes will probably always be fed one by one anyway. Otherwise a faster way of removing a sequence might be needed (one by one is inefficient).";

    static SMALL_TWO: &str = "This adds some function, I guess?, that has a size set at construction, and keeps bytes in a cyclic buffer of that size to be ground up and injested when it fills up.

Currently my sense of smell (and the pain of implementing Write) might be too much, since bytes will probably always be fed one by one anyway. Otherwise crying might be needed (one by one is inefficient).";

    static INTERVAL_STR: &str = include_str!("../src/interval.rs");
    static BREAKS_STR: &str = include_str!("../src/breaks.rs");

    #[test]
    fn diff_smoke_test() {
        let one = SMALL_ONE.into();
        let two = SMALL_TWO.into();

        let delta = LineHashDiff::compute_delta(&one, &two);
        println!("delta: {:?}", &delta);

        let result = delta.apply(&one);
        assert_eq!(result, two);

        let delta = LineHashDiff::compute_delta(&one, &two);
        println!("delta: {:?}", &delta);

        let result = delta.apply(&one);
        assert_eq!(result, two);
    }

    #[test]
    fn test_larger_diff() {
        let one = INTERVAL_STR.into();
        let two = BREAKS_STR.into();

        let delta = LineHashDiff::compute_delta(&one, &two);
        let result = delta.apply(&one);
        assert_eq!(String::from(result), String::from(two));
    }
}
