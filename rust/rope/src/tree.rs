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

//! A general b-tree structure suitable for ropes and the like.

use std::sync::Arc;
use std::ops::{Index, RangeFull};
use std::cmp::{min,max};

const MIN_CHILDREN: usize = 4;
const MAX_CHILDREN: usize = 8;

pub trait NodeInfo: Clone {
    type L : Leaf;
    fn accumulate(&mut self, other: &Self);

    // return info
    fn compute_info(&Self::L) -> Self;

    // default?
}

pub trait Leaf: Sized + Clone + Default {

    // measurement of leaf in base units
    fn len(&self) -> usize;

    // generally a minimum size requirement for leaves
    fn is_ok_child(&self) -> bool;

    // start and end are in "base units"
    // if end == len + 1, then include trailing fragment
    // (note: some leaf types don't have fragments, but still have to
    // deal with len + 1)
    // generally implements a maximum size
    // Invariant: if one or the other input is empty, then no split

    // Invariant: if either input satisfies is_ok_child, then
    // satisfies this, as does the optional split.

    fn push_maybe_split(&mut self, other: &Self, start: usize, end: usize) -> Option<Self>;

    // same meaning as push_maybe_split starting from an empty
    // leaf, but maybe can be implemented more efficiently?
    // TODO: remove if it doesn't pull its weight
    fn subseq(&self, start: usize, end: usize) -> Self {
        let mut result = Self::default();
        if result.push_maybe_split(self, start, end).is_some() {
            panic!("unexpected split");
        }
        result
    }
}

#[derive(Clone)]
pub struct Node<N: NodeInfo>(Arc<NodeBody<N>>);

#[derive(Clone)]
struct NodeBody<N: NodeInfo> {
    height: usize,
    len: usize,
    info: N,
    val: NodeVal<N>,
}

#[derive(Clone)]
enum NodeVal<N: NodeInfo> {
    Leaf(N::L),
    Internal(Vec<Node<N>>),
}

// also consider making Metric a newtype for usize, so type system can
// help separate metrics
pub trait Metric<N: NodeInfo> {
    // probably want len also
    fn measure(&N) -> usize;

    fn to_base_units(l: &N::L, in_measured_units: usize) -> usize;

    fn from_base_units(l: &N::L, in_base_units: usize) -> usize;

    // the next three methods work in base units

    // These methods must indicate a boundary at the end of a leaf,
    // if present. A boundary at the beginning of a leaf is optional
    // (the previous leaf will be queried)

    fn is_boundary(l: &N::L, offset: usize) -> bool;

    // will be called with offset > 0
    fn prev(l: &N::L, offset: usize) -> Option<usize>;

    fn next(l: &N::L, offset: usize) -> Option<usize>;

    fn can_fragment() -> bool;
}

impl<N: NodeInfo> Node<N> {
    pub fn from_leaf(l: N::L) -> Node<N> {
        let len = l.len();
        let info = N::compute_info(&l);
        Node(Arc::new(
            NodeBody {
            height: 0,
            len: len,
            info: info,
            val: NodeVal::Leaf(l),
        }))
    }

    fn from_nodes(nodes: Vec<Node<N>>) -> Node<N> {
        let height = nodes[0].0.height + 1;
        let mut len = nodes[0].0.len;
        let mut info = nodes[0].0.info.clone();
        for child in &nodes[1..] {
            len += child.0.len;
            info.accumulate(&child.0.info);
        }
        Node(Arc::new(
            NodeBody {
            height: height,
            len: len,
            info: info,
            val: NodeVal::Internal(nodes),
        }))
    }

    pub fn len(&self) -> usize {
        self.0.len
    }

    fn height(&self) -> usize {
        self.0.height
    }

    fn get_children(&self) -> &[Node<N>] {
        if let &NodeVal::Internal(ref v) = &self.0.val {
            v
        } else {
            panic!("get_children called on leaf node");
        }
    }

    fn get_leaf(&self) -> &N::L {
        if let &NodeVal::Leaf(ref l) = &self.0.val {
            l
        } else {
            panic!("get_leaf called on internal node");
        }
    }

    fn is_ok_child(&self) -> bool {
        match self.0.val {
            NodeVal::Leaf(ref l) => l.is_ok_child(),
            NodeVal::Internal(ref nodes) => (nodes.len() >= MIN_CHILDREN)
        }
    }

    fn merge_nodes(children1: &[Node<N>], children2: &[Node<N>]) -> Node<N> {
        let n_children = children1.len() + children2.len();
        if n_children <= MAX_CHILDREN {
            Node::from_nodes([children1, children2].concat())
        } else {
            // Note: this leans left. Splitting at midpoint is also an option
            let splitpoint = min(MAX_CHILDREN, n_children - MIN_CHILDREN);
            let mut iter = children1.iter().chain(children2.iter()).cloned();
            let left = iter.by_ref().take(splitpoint).collect();
            let right = iter.collect();
            let parent_nodes = vec![Node::from_nodes(left), Node::from_nodes(right)];
            Node::from_nodes(parent_nodes)
        }
    }

    // precondition: both ropes are leaves
    fn merge_leaves(mut rope1: Node<N>, rope2: Node<N>) -> Node<N> {
        let both_ok = rope1.get_leaf().is_ok_child() && rope2.get_leaf().is_ok_child();
        if both_ok {
            return Node::from_nodes(vec![rope1, rope2]);
        }
        match {
            let mut node1 = Arc::make_mut(&mut rope1.0);
            let leaf2 = rope2.get_leaf();
            let len2 = leaf2.len();
            if let NodeVal::Leaf(ref mut leaf1) = node1.val {
                leaf1.push_maybe_split(leaf2, 0, len2 + 1)
            } else {
                panic!("merge_leaves called on non-leaf");
            }
        } {
            Some(new) => {
                Node::from_nodes(vec![
                    rope1,
                    Node::from_leaf(new),
                ])
            }
            None => rope1
        }
    }

    fn concat(rope1: Node<N>, rope2: Node<N>) -> Node<N> {
        let h1 = rope1.height();
        let h2 = rope2.height();
        if h1 == h2 {
            if rope1.is_ok_child() && rope2.is_ok_child() {
                return Node::from_nodes(vec![rope1, rope2]);
            }
            if h1 == 0 {
                return Node::merge_leaves(rope1, rope2);
            }
            return Node::merge_nodes(rope1.get_children(), rope2.get_children());
        } else if h1 < h2 {
            let children2 = rope2.get_children();
            if h1 == h2 - 1 && rope1.is_ok_child() {
                return Node::merge_nodes(&[rope1], children2);
            }
            let newrope = Node::concat(rope1, children2[0].clone());
            if newrope.height() == h2 - 1 {
                return Node::merge_nodes(&[newrope], &children2[1..]);
            } else {
                return Node::merge_nodes(newrope.get_children(), &children2[1..]);
            }
        } else {  // h1 > h2
            let children1 = rope1.get_children();
            if h2 == h1 - 1 && rope2.is_ok_child() {
                return Node::merge_nodes(children1, &[rope2]);
            }
            let lastix = children1.len() - 1;
            let newrope = Node::concat(children1[lastix].clone(), rope2);
            if newrope.height() == h1 - 1 {
                return Node::merge_nodes(&children1[..lastix], &[newrope]);
            } else {
                return Node::merge_nodes(&children1[..lastix], newrope.get_children());
            }
        }
    }

    fn measure<M: Metric<N>>(&self) -> usize {
        M::measure(&self.0.info)
    }

    fn fudge<M: Metric<N>>(&self) -> usize {
        if M::can_fragment() { 1 } else { 0 }
    }

    // calls the given function with leaves forming the sequence
    fn visit_subseq<M: Metric<N>, F>(&self, start: usize, end: usize,
            f: &mut F) where F: FnMut(&N::L) -> () {
        match self.0.val {
            NodeVal::Leaf(ref l) => {
                if start == 0 && end >= self.measure::<M>() + self.fudge::<M>() {
                    f(&l);
                } else {
                    f(&l.clone().subseq(start, end));
                }
            }
            NodeVal::Internal(ref v) => {
                let mut offset = 0;
                for child in v {
                    if end <= offset {
                        break;
                    }
                    let child_measure = child.measure::<M>();
                    if offset + child_measure > start {
                        child.visit_subseq::<M, F>(max(offset, start) - offset,
                            min(child_measure + child.fudge::<M>(), end - offset), f);
                    }
                    offset += child_measure;
                }
                return;
            }
        }
    }

    fn push_subseq<M: Metric<N>>(&self,
            b: &mut RopeBuilder<N>, start: usize, end: usize) {
        if start == 0 && self.measure::<M>() >= end + self.fudge::<M>() {
            b.push(self.clone());
            return
        }
        match self.0.val {
            NodeVal::Leaf(ref l) => {
                let base_start = M::to_base_units(l, start);
                let base_end = M::to_base_units(l, end);
                b.push_leaf_slice(l, base_start, base_end)
            }
            NodeVal::Internal(ref v) => {
                let mut offset = 0;
                for child in v {
                    if end <= offset {
                        break;
                    }
                    let child_measure = child.measure::<M>();
                    let child_fudged = child_measure + child.fudge::<M>();
                    if offset + child_fudged > start {
                        child.push_subseq::<M>(b,
                            max(offset, start) - offset,
                            min(child_fudged, end - offset));
                    }
                    offset += child_measure;
                }
                return;
            }
        }
    }

    pub fn subseq<M: Metric<N>>(&self, start: usize, end: usize) -> Node<N> {
        let mut b = RopeBuilder::new();
        self.push_subseq::<M>(&mut b, start, end);
        b.build()
    }

    pub fn edit<M: Metric<N>>(&mut self, start: usize, end: usize, new: Node<N>) {
        let mut b = RopeBuilder::new();
        self.push_subseq::<M>(&mut b, 0, start);
        b.push(new);
        self.push_subseq::<M>(&mut b, end, self.measure::<M>() + self.fudge::<M>());
        *self = b.build();
    }

    fn convert_metrics<M1: Metric<N>, M2: Metric<N>>(&self, mut m1: usize) -> usize {
        if m1 == 0 { return 0; }
        if m1 >= self.measure::<M1>() + self.fudge::<M1>() {
            return self.measure::<M2>() + self.fudge::<M2>();
        }
        let mut m2 = 0;
        let mut node = self;
        while node.height() > 0 {
            for child in node.get_children() {
                let child_m1 = child.measure::<M1>();
                if m1 < child_m1 + child.fudge::<M1>() {
                    node = child;
                    break;
                }
                m2 += child.measure::<M2>();
                m1 -= child_m1;
            }
        }
        let l = node.get_leaf();
        let base = M1::to_base_units(l, m1);
        m2 + M2::from_base_units(l, m2)
    }
}

impl<N: NodeInfo> Default for Node<N> {
    fn default() -> Node<N> {
        Node::from_leaf(N::L::default())
    }
}

struct RopeBuilder<N: NodeInfo>(Option<Node<N>>);

impl<N: NodeInfo> RopeBuilder<N> {
    fn new() -> RopeBuilder<N> {
        RopeBuilder(None)
    }

    fn push(&mut self, n: Node<N>) {
        match self.0.take() {
            None => self.0 = Some(n),
            Some(buf) => self.0 = Some(Node::concat(buf, n))
        }
    }

    fn push_leaf(&mut self, l: N::L) {
        self.push(Node::from_leaf(l))
    }

    fn push_leaf_slice(&mut self, l: &N::L, start: usize, end: usize) {
        self.push(Node::from_leaf(l.subseq(start, end)))
    }

    fn build(self) -> Node<N> {
        match self.0 {
            Some(r) => r,
            None => Node::from_leaf(N::L::default())
        }
    }
}

const CURSOR_CACHE_SIZE: usize = 4;

pub struct Cursor<'a, N: 'a + NodeInfo> {
    root: &'a Node<N>,
    position: usize,
    cache: [Option<(&'a Node<N>, usize)>; CURSOR_CACHE_SIZE],
    leaf: Option<&'a N::L>,
    offset_of_leaf: usize,
}

impl<'a, N: NodeInfo> Cursor<'a, N> {
    pub fn new(n: &'a Node<N>, position: usize) -> Cursor<'a, N> {
        let mut result = Cursor {
            root: n,
            position: position,
            cache: [None; CURSOR_CACHE_SIZE],
            leaf: None,
            offset_of_leaf: 0,
        };
        result.descend();
        result
    }

    // return value is leaf (if cursor is valid) and offset within leaf
    // postcondition: offset is at end of leaf iff end of rope
    pub fn get_leaf(&self) -> Option<(&'a N::L, usize)> {
        self.leaf.map(|l| (l, self.position - self.offset_of_leaf))
    }

    pub fn set(&mut self, position: usize) {
        self.position = position;
        // TODO: reuse cache if position is nearby
        self.descend();
    }

    pub fn is_boundary<M: Metric<N>>(&mut self) -> bool {
        if self.leaf.is_none() {
            // not at a valid position
            return false;
        }
        if self.position == 0 || self.position == self.root.len() ||
                (self.position == self.offset_of_leaf && !M::can_fragment()) {
            return true;
        }
        if self.position > self.offset_of_leaf {
            return M::is_boundary(self.leaf.unwrap(),
                self.position - self.offset_of_leaf);
        }
        // tricky case, at beginning of leaf, need to query end of previous
        // leaf; would be nice if we could do it another way that didn't make
        // the method &self mut.
        let l = self.prev_leaf().unwrap().0;
        let result = M::is_boundary(l, l.len());
        let _ = self.next_leaf();
        result
    } 

    pub fn prev<M: Metric<N>>(&mut self) -> Option<(usize)> {
        // TODO: walk up tree to skip measure-0 nodes
        if self.position == 0 {
            self.leaf = None;
            return None;
        }
        loop {
            let mut offset_in_leaf = self.position - self.offset_of_leaf;
            let mut fudge = 0;
            if let Some(l) = self.leaf {
                if offset_in_leaf > 0 {
                    if let Some(offset_in_leaf) = M::prev(l, offset_in_leaf + fudge) {
                        if offset_in_leaf == l.len() {
                            let _ = self.next_leaf();
                            return Some(self.position);
                        }
                        self.position = self.offset_of_leaf + offset_in_leaf;
                        return Some(self.position);
                    }
                    if self.offset_of_leaf == 0 {
                        self.position = 0;
                        return Some(self.position);
                    }
                    fudge = if M::can_fragment() { 1 } else { 0 };
                } else {
                    fudge = 0;
                }
                // needs more refinement
                if let Some((l, _)) = self.prev_leaf() {
                    offset_in_leaf = l.len();
                } else {
                    panic!("inconsistent, shouldn't get here");
                }
            } else {
                panic!("inconsistent, shouldn't get here either");
            }
        }
    }

    pub fn next<M: Metric<N>>(&mut self) -> Option<(usize)> {
        // TODO: walk up tree to skip measure-0 nodes
        if self.position >= self.root.len() {
            self.leaf = None;
            return None;
        }
        loop {
            if let Some(l) = self.leaf {
                let offset_in_leaf = self.position - self.offset_of_leaf;
                if let Some(offset_in_leaf) = M::next(l, offset_in_leaf) {
                    if offset_in_leaf == l.len() &&
                            self.offset_of_leaf + offset_in_leaf != self.root.len() {
                        let _ = self.next_leaf();
                        return Some(self.position);
                    }
                    self.position = self.offset_of_leaf + offset_in_leaf;
                    return Some(self.position);
                }
                if self.offset_of_leaf + l.len() == self.root.len() {
                    self.position = self.root.len();
                    return Some(self.position);
                }
                let _ = Some(self.position);
            } else {
                panic!("inconsistent, shouldn't get here");
            }
        }
    }

    // same return as get_leaf, moves to beginning of next leaf
    // make pub?
    fn next_leaf(&mut self) -> Option<(&'a N::L, usize)> {
        if let Some(leaf) = self.leaf {
            self.position = self.offset_of_leaf + leaf.len();
        } else {
            return None;
        }
        for i in 0..CURSOR_CACHE_SIZE {
            if self.cache[i].is_none() {
                return None;
            }
            let (node, j) = self.cache[i].unwrap();
            if j + 1 < node.get_children().len() {
                self.cache[i] = Some((node, j + 1));
                let mut node_down = &node.get_children()[j + 1];
                for k in (0..i).rev() {
                    self.cache[k] = Some((node_down, 0));
                    node_down = &node_down.get_children()[0];
                }
                self.leaf = Some(node_down.get_leaf());
                self.offset_of_leaf = self.position;
                return self.get_leaf();
            }
        }
        self.descend();
        self.get_leaf()
    }

    // same return as get_leaf, moves to beginning of prev leaf
    // make pub?
    fn prev_leaf(&mut self) -> Option<(&'a N::L, usize)> {
        if self.offset_of_leaf == 0 || Some(self.leaf).is_none() {
            return None;
        }
        for i in 0..CURSOR_CACHE_SIZE {
            if self.cache[i].is_none() {
                return None;
            }
            let (node, j) = self.cache[i].unwrap();
            if j > 0 {
                self.cache[i] = Some((node, j - 1));
                let mut node_down = &node.get_children()[j - 1];
                for k in (0..i).rev() {
                    let last_ix = node_down.get_children().len() - 1;
                    self.cache[k] = Some((node_down, last_ix));
                    node_down = &node_down.get_children()[last_ix];
                }
                let leaf = node_down.get_leaf();
                self.leaf = Some(leaf);
                self.offset_of_leaf -= leaf.len();
                self.position = self.offset_of_leaf;
                return self.get_leaf();
            }
        }
        self.position = self.offset_of_leaf - 1;
        self.descend();
        self.position = self.offset_of_leaf;
        self.get_leaf()
    }

    fn descend(&mut self) {
        let mut node = self.root;
        let mut offset = 0;
        while node.height() > 0 {
            let children = node.get_children();
            let mut i = 0;
            loop {
                if i == children.len() {
                    self.leaf = None;
                    return;
                }
                let nextoff = offset + children[i].len();
                if nextoff > self.position {
                    break;
                }
                offset = nextoff;
                i += 1;
            }
            let cache_ix = node.height() - 1;
            if cache_ix < CURSOR_CACHE_SIZE {
                self.cache[cache_ix] = Some((node, i));
            }
            node = &children[i];
        }
        self.leaf = Some(node.get_leaf());
        self.offset_of_leaf = offset;
    }
}

/*

// How to access the slice type for a leaf, if available. This will
// be super helpful in building a chunk iterator (which requires
// slices if it's going to conform to Rust's iterator protocol)
fn slice<'a, L: Leaf + Index<RangeFull>>(l: &'a L) -> &'a L::Output {
    l.index(RangeFull)
}
*/

// some concrete instantiations of the traits

#[derive(Clone, Default)]
struct BytesLeaf(Vec<u8>);

#[derive(Clone)]
struct BytesInfo(usize);

// leaf doesn't have to be a newtype
impl Leaf for Vec<u8> {
    fn len(&self) -> usize {
        self.len()
    }

    fn is_ok_child(&self) -> bool {
        self.len() >= 512
    }

    fn push_maybe_split(&mut self, other: &Vec<u8>, start: usize, end: usize) -> Option<Vec<u8>> {
        self.extend_from_slice(&other[start..end]);
        if self.len() <= 1024 {
            None
        } else {
            let splitpoint = self.len() / 2;
            let new = self[splitpoint..].to_owned();
            self.truncate(splitpoint);
            Some(new)
        }
    }
}

impl NodeInfo for BytesInfo {
    type L = Vec<u8>;
    fn accumulate(&mut self, other: &Self) {
        self.0 += other.0;
    }

    fn compute_info(l: &Vec<u8>) -> BytesInfo {
        BytesInfo(l.len())
    }
}

struct BytesMetric(());

impl Metric<BytesInfo> for BytesMetric {
    fn measure(info: &BytesInfo) -> usize {
        info.0
    }

    fn to_base_units(_: &Vec<u8>, in_measured_units: usize) -> usize {
        in_measured_units
    }

    fn from_base_units(_: &Vec<u8>, in_base_units: usize) -> usize {
        in_base_units
    }

    fn is_boundary(_: &Vec<u8>, offset: usize) -> bool { true }

    fn prev(_: &Vec<u8>, offset: usize) -> Option<usize> {
        if offset > 0 { Some(offset - 1) } else { None }
    }

    fn next(l: &Vec<u8>, offset: usize) -> Option<usize> {
        if offset < l.len() { Some(offset + 1) } else { None }
    }

    fn can_fragment() -> bool { false }
}
