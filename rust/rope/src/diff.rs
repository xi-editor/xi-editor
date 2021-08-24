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

use std::borrow::Cow;
use std::collections::HashMap;

use crate::compare::RopeScanner;
use crate::delta::{Delta, DeltaElement};
use crate::interval::Interval;
use crate::rope::{LinesMetric, Rope, RopeDelta, RopeInfo};
use crate::tree::{Node, NodeInfo};

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

        // if a continuous range of text got deleted, we're done
        if target.len() < base.len() && start_offset + diff_end == target.len() {
            builder.copy(base.len() - diff_end, target_end, diff_end);
            return builder.to_delta(base, target);
        }

        // if a continuous range of text got inserted, we're done
        if target.len() > base.len() && start_offset + diff_end == base.len() {
            builder.copy(base.len() - diff_end, target_end, diff_end);
            return builder.to_delta(base, target);
        }

        let line_hashes = make_line_hashes(base, MIN_SIZE);

        let line_count = target.measure::<LinesMetric>() + 1;
        let mut matches = Vec::with_capacity(line_count);

        let mut targ_line_offset = 0;
        let mut prev_base = 0;

        let mut needs_subseq = false;
        for line in target.lines_raw(start_offset..target_end) {
            let non_ws = non_ws_offset(&line);
            if line.len() - non_ws >= MIN_SIZE {
                if let Some(base_off) = line_hashes.get(&line[non_ws..]) {
                    let targ_off = targ_line_offset + non_ws;
                    matches.push((start_offset + targ_off, *base_off));
                    if *base_off < prev_base {
                        needs_subseq = true;
                    }
                    prev_base = *base_off;
                }
            }
            targ_line_offset += line.len();
        }

        // we now have an ordered list of matches and their positions.
        // to ensure that our delta only copies non-decreasing base regions,
        // we take the longest increasing subsequence.
        // TODO: a possible optimization here would be to expand matches
        // to adjacent lines first? this would be at best a small win though..

        let longest_subseq =
            if needs_subseq { longest_increasing_region_set(&matches) } else { matches };

        // for each matching region, we extend it forwards and backwards.
        // we keep track of how far forward we extend it each time, to avoid
        // having a subsequent scan extend backwards over the same region.
        let mut prev_end = start_offset;

        for (targ_off, base_off) in longest_subseq {
            if targ_off <= prev_end {
                continue;
            }
            let (left_dist, mut right_dist) =
                expand_match(base, target, base_off, targ_off, prev_end);

            // don't let last match expand past target_end
            right_dist = right_dist.min(target_end - targ_off);

            let targ_start = targ_off - left_dist;
            let base_start = base_off - left_dist;
            let len = left_dist + right_dist;
            prev_end = targ_start + len;

            builder.copy(base_start, targ_start, len);
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
fn expand_match(
    base: &Rope,
    target: &Rope,
    base_off: usize,
    targ_off: usize,
    prev_match_targ_end: usize,
) -> (usize, usize) {
    let mut scanner = RopeScanner::new(base, target);
    let max_left = targ_off - prev_match_targ_end;
    let start = scanner.find_ne_char_back(base_off, targ_off, max_left);
    debug_assert!(start <= max_left, "{} <= {}", start, max_left);
    let end = scanner.find_ne_char(base_off, targ_off, None);
    (start.min(max_left), end)
}

/// Finds the longest increasing subset of copyable regions. This is essentially
/// the longest increasing subsequence problem. This implementation is adapted
/// from https://codereview.stackexchange.com/questions/187337/longest-increasing-subsequence-algorithm
fn longest_increasing_region_set(items: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut result = vec![0];
    let mut prev_chain = vec![0; items.len()];

    for i in 1..items.len() {
        // If the next item is greater than the last item of the current longest
        // subsequence, push its index at the end of the result and continue.
        let last_idx = *result.last().unwrap();
        if items[last_idx].1 < items[i].1 {
            prev_chain[i] = last_idx;
            result.push(i);
            continue;
        }

        let next_idx = match result.binary_search_by(|&j| items[j].1.cmp(&items[i].1)) {
            Ok(_) => continue, // we ignore duplicates
            Err(idx) => idx,
        };

        if items[i].1 < items[result[next_idx]].1 {
            if next_idx > 0 {
                prev_chain[i] = result[next_idx - 1];
            }
            result[next_idx] = i;
        }
    }

    // walk backwards from the last item in result to build the final sequence
    let mut u = result.len();
    let mut v = *result.last().unwrap();
    while u != 0 {
        u -= 1;
        result[u] = v;
        v = prev_chain[v];
    }
    result.iter().map(|i| items[*i]).collect()
}

#[inline]
fn non_ws_offset(s: &str) -> usize {
    s.as_bytes().iter().take_while(|b| **b == b' ' || **b == b'\t').count()
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
        self.ops.push(DiffOp { target_idx: target, base_idx: base, len })
    }

    fn to_delta(self, base: &Rope, target: &Rope) -> RopeDelta {
        let mut els = Vec::with_capacity(self.ops.len() * 2);
        let mut targ_pos = 0;
        for DiffOp { base_idx, target_idx, len } in self.ops {
            if target_idx > targ_pos {
                let iv = Interval::new(targ_pos, target_idx);
                els.push(DeltaElement::Insert(target.subseq(iv)));
            }
            els.push(DeltaElement::Copy(base_idx, base_idx + len));
            targ_pos = target_idx + len;
        }

        if targ_pos < target.len() {
            let iv = Interval::new(targ_pos, target.len());
            els.push(DeltaElement::Insert(target.subseq(iv)));
        }

        Delta { els, base_len: base.len() }
    }
}

/// Creates a map of lines to offsets, ignoring trailing whitespace, and only for those lines
/// where line.len() >= min_size. Offsets refer to the first non-whitespace byte in the line.
fn make_line_hashes(base: &Rope, min_size: usize) -> HashMap<Cow<str>, usize> {
    let mut offset = 0;
    let mut line_hashes = HashMap::with_capacity(base.len() / 60);
    for line in base.lines_raw(..) {
        let non_ws = non_ws_offset(&line);
        if line.len() - non_ws >= min_size {
            let cow = match line {
                Cow::Owned(ref s) => Cow::Owned(s[non_ws..].to_string()),
                Cow::Borrowed(s) => Cow::Borrowed(&s[non_ws..]),
            };
            line_hashes.insert(cow, offset + non_ws);
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
    fn simple_diff() {
        let one = "This is a simple string".into();
        let two = "This is a string".into();

        let delta = LineHashDiff::compute_delta(&one, &two);
        println!("delta: {:?}", &delta);

        let result = delta.apply(&one);
        assert_eq!(result, two);

        let delta = LineHashDiff::compute_delta(&two, &one);
        println!("delta: {:?}", &delta);

        let result = delta.apply(&two);
        assert_eq!(result, one);
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
