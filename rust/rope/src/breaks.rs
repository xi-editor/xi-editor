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

//! A module for representing a set of breaks, typically used for
//! storing the result of line breaking.

use std::cmp::min;
use std::mem;
use tree::{Node, Leaf, NodeInfo, Metric, TreeBuilder};
use interval::Interval;

// Breaks represents a set of indexes. A motivating use is storing line breaks.
pub type Breaks = Node<BreaksInfo>;

// Here the base units are arbitrary, but most commonly match the base units
// of the rope storing the underlying string.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BreaksLeaf {
    len: usize,  // measured in base units
    data: Vec<usize>,  // each is a delta relative to start of leaf; sorted
}

#[derive(Clone)]
pub struct BreaksInfo(usize);  // number of breaks

impl Leaf for BreaksLeaf {
    fn len(&self) -> usize {
        self.len
    }

    fn is_ok_child(&self) -> bool {
        self.data.len() >= 32
    }

    fn push_maybe_split(&mut self, other: &BreaksLeaf, iv: Interval) -> Option<BreaksLeaf> {
        //eprintln!("push_maybe_split {:?} {:?} {}", self, other, iv);
        let (start, end) = iv.start_end();
        let start_test = if iv.is_start_closed() { start } else { start + 1 };
        for &v in &other.data {
            if start_test <= v && v <= end {
                self.data.push(v - start + self.len);
            }
        }
        // the min with other.len() shouldn't be needed
        self.len += min(end, other.len()) - start;

        if self.data.len() <= 64 {
            None
        } else {
            let splitpoint = self.data.len() / 2;  // number of breaks
            let splitpoint_units = self.data[splitpoint - 1];
            // TODO: use Vec::split_off(), it's nicer
            let mut new = Vec::with_capacity(self.data.len() - splitpoint);
            for i in splitpoint..self.data.len() {
                new.push(self.data[i] - splitpoint_units);
            }
            let new_len = self.len - splitpoint_units;
            self.len = splitpoint_units;
            self.data.truncate(splitpoint);
            Some(BreaksLeaf {
                len: new_len,
                data: new,
            })
        }
    }
}

impl NodeInfo for BreaksInfo {
    type L = BreaksLeaf;

    fn accumulate(&mut self, other: &Self) {
        self.0 += other.0;
    }

    fn compute_info(l: &BreaksLeaf) -> BreaksInfo {
        BreaksInfo(l.data.len())
    }
}

#[derive(Copy, Clone)]
pub struct BreaksMetric(());

impl Metric<BreaksInfo> for BreaksMetric {
    fn measure(info: &BreaksInfo, _: usize) -> usize {
        info.0
    }

    fn to_base_units(l: &BreaksLeaf, in_measured_units: usize) -> usize {
        if in_measured_units > l.data.len() {
            l.len + 1
        } else if in_measured_units == 0 {
            0
        } else {
            l.data[in_measured_units - 1]
        }
    }

    fn from_base_units(l: &BreaksLeaf, in_base_units: usize) -> usize {
        // TODO: binary search, data is sorted
        for i in 0..l.data.len() {
            if in_base_units < l.data[i] {
                return i;
            }
        }
        l.data.len()
    }

    fn is_boundary(l: &BreaksLeaf, offset: usize) -> bool {
        // TODO: binary search, data is sorted
        for i in 0..l.data.len() {
            if offset == l.data[i] {
                return true;
            } else if offset < l.data[i] {
                return false;
            }
        }
        false
    }

    fn prev(l: &BreaksLeaf, offset: usize) -> Option<usize> {
        for i in 0..l.data.len() {
            if offset <= l.data[i] {
                if i == 0 {
                    return None;
                } else {
                    return Some(l.data[i - 1]);
                }
            }
        }
        l.data.last().cloned()
    }

    fn next(l: &BreaksLeaf, offset: usize) -> Option<usize> {
        // TODO: binary search, data is sorted
        for i in 0..l.data.len() {
            if offset < l.data[i] {
                return Some(l.data[i]);
            }
        }
        None
    }

    fn can_fragment() -> bool { true }
}

#[derive(Copy, Clone)]
pub struct BreaksBaseMetric(());

impl Metric<BreaksInfo> for BreaksBaseMetric {
    fn measure(_: &BreaksInfo, len: usize) -> usize {
        len
    }

    fn to_base_units(_: &BreaksLeaf, in_measured_units: usize) -> usize {
        in_measured_units
    }

    fn from_base_units(_: &BreaksLeaf, in_base_units: usize) -> usize {
        in_base_units
    }

    fn is_boundary(l: &BreaksLeaf, offset: usize) -> bool {
        BreaksMetric::is_boundary(l, offset)
    }

    fn prev(l: &BreaksLeaf, offset: usize) -> Option<usize> {
        BreaksMetric::prev(l, offset)
    }

    fn next(l: &BreaksLeaf, offset: usize) -> Option<usize> {
        BreaksMetric::next(l, offset)
    }

    fn can_fragment() -> bool { true }
}

// Additional functions specific to breaks

impl Breaks {
    // a length with no break, useful in edit operations; for
    // other use cases, use the builder.
    pub fn new_no_break(len: usize) -> Breaks {
        let leaf = BreaksLeaf {
            len,
            data: vec![],
        };
        Node::from_leaf(leaf)
    }
}

pub struct BreakBuilder {
    b: TreeBuilder<BreaksInfo>,
    leaf: BreaksLeaf,
}

impl Default for BreakBuilder {
    fn default() -> BreakBuilder {
        BreakBuilder {
            b: TreeBuilder::new(),
            leaf: BreaksLeaf::default(),
        }
    }
}

impl BreakBuilder {
    pub fn new() -> BreakBuilder {
        BreakBuilder::default()
    }

    pub fn add_break(&mut self, len: usize) {
        if self.leaf.data.len() == 64 {
            let leaf = mem::replace(&mut self.leaf, BreaksLeaf::default());
            self.b.push(Node::from_leaf(leaf));
        }
        self.leaf.len += len;
        self.leaf.data.push(self.leaf.len);
    }

    pub fn add_no_break(&mut self, len: usize) {
        self.leaf.len += len;
    }

    pub fn build(mut self) -> Breaks {
        self.b.push(Node::from_leaf(self.leaf));
        self.b.build()
    }
}

#[cfg(test)]
mod tests {
    use breaks::{BreaksLeaf, BreaksInfo, BreaksMetric, BreakBuilder};
    use tree::{Node, Cursor};
    use interval::Interval;

    fn gen(n: usize) -> Node<BreaksInfo> {
        let mut node = Node::default();
        let mut b = BreakBuilder::new();
        b.add_break(10);
        let testnode = b.build();
        if n == 1 {
            return testnode;
        }
        for _ in 0..n {
            let len = node.len();
            let empty_interval_at_end = Interval::new_open_closed(len, len);
            node.edit(empty_interval_at_end, testnode.clone());
        }
        node
    }

    #[test]
    fn empty() {
        let n = gen(0);
        assert_eq!(0, n.len());
    }

    #[test]
    fn fromleaf() {
        let testnode = gen(1);
        assert_eq!(10, testnode.len());
    }

    #[test]
    fn one() {
        let testleaf = BreaksLeaf {
            len: 10,
            data: vec![10],
        };
        let testnode = Node::<BreaksInfo>::from_leaf(testleaf.clone());
        assert_eq!(10, testnode.len());
        let mut c = Cursor::new(&testnode, 0);
        assert_eq!(c.get_leaf().unwrap().0, &testleaf);
        assert_eq!(10, c.next::<BreaksMetric>().unwrap());
        assert!(c.next::<BreaksMetric>().is_none());
        c.set(0);
        assert!(c.is_boundary::<BreaksMetric>());
        c.set(1);
        assert!(!c.is_boundary::<BreaksMetric>());
        c.set(10);
        assert!(c.is_boundary::<BreaksMetric>());
        assert_eq!(0, c.prev::<BreaksMetric>().unwrap());
        assert!(c.prev::<BreaksMetric>().is_none());
    }

    #[test]
    fn concat() {
        let left = gen(1);
        let right = gen(1);
        let node = Node::concat(left.clone(), right);
        assert_eq!(node.len(), 20);
        let mut c = Cursor::new(&node, 0);
        assert_eq!(10, c.next::<BreaksMetric>().unwrap());
        assert_eq!(20, c.next::<BreaksMetric>().unwrap());
        assert!(c.next::<BreaksMetric>().is_none());
    }

    #[test]
    fn larger() {
        let node = gen(100);
        assert_eq!(node.len(), 1000);
    }
}
