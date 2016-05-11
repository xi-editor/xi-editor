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

//! A data structure for representing editing operations on ropes.
//! It's useful to explicitly represent these operations so they can be
//! shared across multiple subsystems.

use interval::Interval;
use tree::{Node, NodeInfo, TreeBuilder};
use subset::{Subset, SubsetBuilder};
use std;

pub enum DeltaElement<N: NodeInfo> {
    Copy(usize, usize),  // note: for now, we lose open/closed info at interval endpoints
    Insert(Node<N>),
}

// A case can be made this should also include the length; then it could be a debug
// assert to apply to a rope of the correct length.
pub struct Delta<N: NodeInfo>(Vec<DeltaElement<N>>);

impl<N: NodeInfo> Delta<N> {
    pub fn simple_edit(interval: Interval, rope: Node<N>, base_len: usize) -> Delta<N> {
        let mut result = Vec::new();
        let (start, end) = interval.start_end();
        if start > 0 {
            result.push(DeltaElement::Copy(0, start));
        }
        if rope.len() > 0 {
            result.push(DeltaElement::Insert(rope));
        }
        if end < base_len {
            result.push(DeltaElement::Copy(end, base_len));
        }
        Delta(result)
    }

    /// Apply the delta to the given rope. May not work well if the length of the rope
    /// is not compatible with the construction of the delta.
    pub fn apply(&self, base: & Node<N>) -> Node<N> {
        let mut b = TreeBuilder::new();
        for elem in &self.0 {
            match *elem {
                DeltaElement::Copy(beg, end) => {
                    base.push_subseq(&mut b, Interval::new_closed_open(beg, end))
                }
                DeltaElement::Insert(ref n) => b.push(n.clone())
            }
        }
        b.build()
    }

    /// Factor the delta into an insert-only delta and a subset representing deletions.
    /// Applying the insert then the delete yields the same result as the original delta:
    ///
    /// `let (d1, ss) = d.factor();`
    ///
    /// `ss.apply_to_string(d1.apply_to_string(s)) == d.apply_to_string(s)`
    pub fn factor(self, len: usize) -> (Delta<N>, Subset) {
        let mut ins = Vec::new();
        let mut sb = SubsetBuilder::new();
        let mut b1 = 0;
        let mut e1 = 0;
        let mut delta = 0;
        for elem in self.0 {
            match elem {
                DeltaElement::Copy(b, e) => {
                    sb.add_deletion(e1 + delta, b + delta);
                    e1 = e;
                }
                DeltaElement::Insert(n) => {
                    if e1 > b1 {
                        ins.push(DeltaElement::Copy(b1, e1));
                    }
                    b1 = e1;
                    delta += n.len();
                    ins.push(DeltaElement::Insert(n));
                }
            }
        }
        if b1 < len {
            ins.push(DeltaElement::Copy(b1, len));
        }
        sb.add_deletion(e1 + delta, len + delta);
        (Delta(ins), sb.build())
    }

    /// Return a subset that inverts the insert-only delta:
    ///
    /// `d.invert_insert().apply_to_string(d.apply_to_string(s)) == s`
    pub fn invert_insert(&self) -> Subset {
        let mut sb = SubsetBuilder::new();
        let mut x = 0;
        for elem in &self.0 {
            match *elem {
                DeltaElement::Copy(b, e) => {
                    x += e - b;
                }
                DeltaElement::Insert(ref n) => {
                    sb.add_deletion(x, x + n.len());
                    x += n.len();
                }
            }
        }
        sb.build()
    }
}

// This version of the Delta data structure will be replaced by the new one,
// as it's not as suitable for async updates and undo. We keep it until the
// new one is ready to use.
pub struct OldDelta<N: NodeInfo> {
    items: Vec<DeltaItem<N>>,
}

pub struct DeltaItem<N: NodeInfo> {
    pub interval: Interval,
    pub rope: Node<N>,
}

pub type Iter<'a, N> = std::slice::Iter<'a, DeltaItem<N>>;

impl<N: NodeInfo> OldDelta<N> {
    pub fn new() -> OldDelta<N> {
        OldDelta {
            items: Vec::new(),
        }
    }

    pub fn add(&mut self, interval: Interval, rope: Node<N>) {
        self.items.push(DeltaItem {
            interval: interval,
            rope: rope,
        })
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> Iter<N> {
        self.items.iter()
    }

    pub fn apply(&self, base: &mut Node<N>) {
        for item in self.iter() {
            base.edit(item.interval, item.rope.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use rope::{Rope, RopeInfo};
    use delta::{Delta};
    use interval::Interval;

    impl Delta<RopeInfo> {
        fn apply_to_string(&self, s: &str) -> String {
            String::from(self.apply(&Rope::from(s)))
        }
    }

    #[test]
    fn simple() {
        let d = Delta::simple_edit(Interval::new_closed_open(1, 9), Rope::from("era"), 11);
        assert_eq!("herald", d.apply_to_string("hello world"));
    }

    #[test]
    fn factor() {
        let d = Delta::simple_edit(Interval::new_closed_open(1, 9), Rope::from("era"), 11);
        let (d1, ss) = d.factor(11);
        assert_eq!("heraello world", d1.apply_to_string("hello world"));
        assert_eq!("herald", ss.apply_to_string("heraello world"));
    }

    #[test]
    fn invert_insert() {
        let d = Delta::simple_edit(Interval::new_closed_open(1, 9), Rope::from("era"), 11);
        let (d1, _ss) = d.factor(11);
        assert_eq!("hello world", d1.invert_insert().apply_to_string("heraello world"));
    }
}
