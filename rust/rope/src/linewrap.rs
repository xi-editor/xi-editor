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

//! A module for representing the result of line breaking.

use std::cmp::min;
use tree::{Node, Leaf, NodeInfo, Metric};

// Another more interesting example - Points represents a (multi-) set
// of indexes. A motivating use is storing line breaks.

// Here the base units are the underlying indices, ie it should track
// the buffer being broken

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PointsLeaf {
    len: usize,  // measured in base units
    data: Vec<usize>,  // each is a delta relative to start of leaf; sorted
}

#[derive(Clone)]
struct PointsInfo(usize);  // number of breaks

impl Leaf for PointsLeaf {
    fn len(&self) -> usize {
        self.len
    }

    fn is_ok_child(&self) -> bool {
        self.data.len() >= 32
    }

    fn push_maybe_split(&mut self, other: &PointsLeaf, start: usize, end: usize) -> Option<PointsLeaf> {
        for &v in other.data.iter() {
            if start <= v && v < end {
                self.data.push(v - start + self.len);
            }
        }
        self.len += min(end, other.len()) - start;

        if self.data.len() <= 64 {
            None
        } else {
            let splitpoint = self.data.len() / 2;  // number of breaks
            let splitpoint_units = self.data[splitpoint - 1];
            let mut new = Vec::with_capacity(self.data.len() - splitpoint);
            for i in splitpoint..self.data.len() {
                new.push(self.data[i] - splitpoint_units);
            }
            let new_len = self.len - splitpoint_units;
            self.len = splitpoint_units;
            self.data.truncate(splitpoint);
            Some(PointsLeaf {
                len: new_len,
                data: new,
            })
        }
    }
}

impl NodeInfo for PointsInfo {
	type L = PointsLeaf;
    fn accumulate(&mut self, other: &Self) {
        self.0 += other.0;
    }

    fn compute_info(l: &PointsLeaf) -> PointsInfo {
        PointsInfo(l.len)
    }
}

struct PointsMetric(());

impl Metric<PointsInfo> for PointsMetric {
    fn measure(info: &PointsInfo) -> usize {
        info.0
    }

    fn to_base_units(l: &PointsLeaf, in_measured_units: usize) -> usize {
        if in_measured_units > l.data.len() {
            l.len + 1
        } else if in_measured_units == 0 {
            0
        } else {
            l.data[in_measured_units - 1]
        }
    }

    fn from_base_units(l: &PointsLeaf, in_base_units: usize) -> usize {
        // TODO: binary search, data is sorted
        for i in 0..l.data.len() {
            if in_base_units < l.data[i] {
                if i == 0 {
                    return 0;  // not satisfying, should be option?
                } else {
                    return l.data[i - 1];
                }
            }
        }
        *l.data.last().unwrap_or(&0)
    }

    fn is_boundary(l: &PointsLeaf, offset: usize) -> bool {
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

    fn prev(l: &PointsLeaf, offset: usize) -> Option<usize> {
        for i in 0..l.data.len() {
            if offset <= l.data[i] {
                if i == 0 {
                    return None;
                } else {
                    return Some(l.data[i - 1]);
                }
            }
        }
        l.data.last().map(|&offset| offset)
    }

    fn next(l: &PointsLeaf, offset: usize) -> Option<usize> {
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

#[cfg(test)]
mod tests {
	use linewrap::{PointsLeaf, PointsInfo, PointsMetric};
	use tree::{Node, Cursor};

	fn gen(n: usize) -> Node<PointsInfo> {
		let mut node = Node::default();
		let testleaf = PointsLeaf {
			len: 10,
			data: vec![10],
		};
		let testnode = Node::<PointsInfo>::from_leaf(testleaf);
		if n == 1 {
			return testnode;
		}
		for _ in 0..n {
			let len = node.len();
			node.edit::<PointsMetric>(len, len, testnode.clone());
		}
		node
	}

	#[test]
	fn empty() {
		let n = gen(0);
		assert_eq!(0, n.len());
	}

	fn fromleaf() {
		let testnode = gen(1);
		assert_eq!(10, testnode.len());
	}

	#[test]
	fn one() {
		let testleaf = PointsLeaf {
			len: 10,
			data: vec![10],
		};
		let testnode = Node::<PointsInfo>::from_leaf(testleaf.clone());
		assert_eq!(10, testnode.len());
		let mut c = Cursor::new(&testnode, 0);
		assert_eq!(c.get_leaf().unwrap().0, &testleaf);
		assert_eq!(10, c.next::<PointsMetric>().unwrap());
		assert!(c.next::<PointsMetric>().is_none());
		c.set(0);
		assert!(c.is_boundary::<PointsMetric>());
		c.set(1);
		assert!(!c.is_boundary::<PointsMetric>());
		c.set(10);
		assert!(c.is_boundary::<PointsMetric>());
		assert_eq!(0, c.prev::<PointsMetric>().unwrap());
		assert!(c.prev::<PointsMetric>().is_none());
	}

	#[test]
	fn concat() {
		let left = gen(1);
		let right = gen(1);
		let node = Node::concat(left.clone(), right);
		assert_eq!(node.len(), 20);
		let mut c = Cursor::new(&node, 0);
		assert_eq!(10, c.next::<PointsMetric>().unwrap());
		assert_eq!(20, c.next::<PointsMetric>().unwrap());
		assert!(c.next::<PointsMetric>().is_none());
	}

	#[test]
	fn larger() {
		let node = gen(100);
		assert_eq!(node.len(), 1000);
	}
}
