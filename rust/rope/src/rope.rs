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

//! A rope data structure with a line count metric and (soon) other useful
//! info.

use std::cmp::{min,max};

use tree::{Leaf, Node, NodeInfo, Metric};
use interval::Interval;

const MIN_LEAF: usize = 511;
const MAX_LEAF: usize = 1024;

/// The main rope data structure. It is implemented as a b-tree with simply
/// `String` as the leaf type. The base metric counts UTF-8 code units
/// (bytes) and has boundaries at code points.
pub type Rope = Node<RopeInfo>;

impl Leaf for String {
    fn len(&self) -> usize {
        self.len()
    }

    fn is_ok_child(&self) -> bool {
        self.len() >= MIN_LEAF
    }

    fn push_maybe_split(&mut self, other: &String, iv: Interval) -> Option<String> {
        let (start, end) = iv.start_end();
        self.push_str(&other[start..end]);
        if self.len() <= MAX_LEAF {
            None
        } else {
            let splitpoint = find_leaf_split_for_merge(self);
            let right_str = self[splitpoint..].to_owned();
            self.truncate(splitpoint);
            self.shrink_to_fit();
            Some(right_str)
        }
    }
}

#[derive(Clone)]
pub struct RopeInfo {
    lines: usize,
}

impl NodeInfo for RopeInfo {
    type L = String;

    type BaseMetric = BaseMetric;

    fn accumulate(&mut self, other: &Self) {
        self.lines += other.lines;
    }

    fn compute_info(s: &String) -> Self {
        RopeInfo {
            lines: count_newlines(s),
        }
    }

    fn identity() -> Self {
        RopeInfo {
            lines: 0,
        }
    }
}

// TODO: explore ways to make this faster - SIMD would be a big win
fn count_newlines(s: &str) -> usize {
    s.as_bytes().iter().filter(|&&c| c == b'\n').count()
}

// TODO: probably will be stabilized in Rust std lib
// Note, this isn't exactly the same, it panics when index > s.len()
fn is_char_boundary(s: &str, index: usize) -> bool {
    // fancy bit magic for ranges 0..0x80 | 0xc0..
    index == s.len() || (s.as_bytes()[index] as i8) >= -0x40
}

/*
// TODO: use this for bulk load
fn find_leaf_split_for_bulk(s: &str) -> usize {
    find_leaf_split(s, MIN_LEAF)
}
*/

fn find_leaf_split_for_merge(s: &str) -> usize {
    find_leaf_split(s, max(MIN_LEAF, s.len() - MAX_LEAF))
}

// Try to split at newline boundary (leaning left), if not, then split at codepoint
fn find_leaf_split(s: &str, minsplit: usize) -> usize {
    let mut splitpoint = min(MAX_LEAF, s.len() - MIN_LEAF);
    match s.as_bytes()[minsplit - 1..splitpoint].iter().rposition(|&c| c == b'\n') {
        Some(pos) => minsplit + pos,
        None => {
            while !is_char_boundary(s, splitpoint) {
                splitpoint -= 1;
            }
            splitpoint
        }
    }
}

pub struct BaseMetric(());

impl Metric<RopeInfo> for BaseMetric {
    fn measure(_: &RopeInfo, len: usize) -> usize {
        len
    }

    fn to_base_units(_: &String, in_measured_units: usize) -> usize {
        in_measured_units
    }

    fn from_base_units(_: &String, in_base_units: usize) -> usize {
        in_base_units
    }

    fn is_boundary(s: &String, offset: usize) -> bool {
        is_char_boundary(s, offset)
    }

    fn prev(s: &String, offset: usize) -> Option<usize> {
        if offset == 0 {
            // I think it's a precondition that this will never be called
            // with offset == 0, but be defensive.
            None
        } else {
            let mut len = 1;
            while !is_char_boundary(s, offset + 1 - len) {
                len += 1;
            }
            Some(offset - len)
        }
    }

    fn next(s: &String, offset: usize) -> Option<usize> {
        if offset == s.len() {
            // I think it's a precondition that this will never be called
            // with offset == s.len(), but be defensive.
            None
        } else {
            let b = s.as_bytes()[offset];
            let len = match b {
                b if b < 0x80 => 1,
                b if b < 0xe0 => 2,
                b if b < 0xf0 => 3,
                _ => 4
            };
            Some(offset + len)
        }
    }

    fn can_fragment() -> bool {
        false
    }
}

struct LinesMetric(usize);  // number of lines

impl Metric<RopeInfo> for LinesMetric {
    fn measure(info: &RopeInfo, _: usize) -> usize {
        info.lines
    }

    fn is_boundary(s: &String, offset: usize) -> bool {
        if offset == 0 {
            // shouldn't be called with this, but be defensive
            false
        } else {
            s.as_bytes()[offset - 1] == b'\n'
        }
    }

    fn to_base_units(s: &String, in_measured_units: usize) -> usize {
        let mut offset = 0;
        for _ in 0..in_measured_units {
            match s[offset..].as_bytes().iter().position(|&c| c == b'\n') {
                Some(pos) => offset += pos + 1,
                _ => panic!("to_base_units called with arg too large")
            }
        }
        offset
    }

    fn from_base_units(s: &String, in_base_units: usize) -> usize {
        count_newlines(&s[..in_base_units])
    }

    fn prev(s: &String, offset: usize) -> Option<usize> {
        s.as_bytes()[..offset].iter().rposition(|&c| c == b'\n')
            .map(|pos| pos + 1)
    }

    fn next(s: &String, offset: usize) -> Option<usize> {
        s.as_bytes()[offset..].iter().position(|&c| c == b'\n')
            .map(|pos| offset + pos + 1)
    }

    fn can_fragment() -> bool { true }
}
