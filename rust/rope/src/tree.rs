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
use std::cmp::min;

use interval::Interval;

const MIN_CHILDREN: usize = 4;
const MAX_CHILDREN: usize = 8;

pub trait NodeInfo: Clone {
    /// The type of the leaf.
    ///
    /// A given NodeInfo is for exactly one type of leaf. That is why
    /// the leaf type is an associated type rather than a type parameter.
    type L : Leaf;

    /// An operator that combines info from two subtrees. It is intended
    /// (but not strictly enforced) that this operator be associative and
    /// obey an identity property. In mathematical terms, the accumulate
    /// method is the sum operator of a monoid.
    fn accumulate(&mut self, other: &Self);

    /// A mapping from a leaf into the info type. It is intended (but
    /// not strictly enforced) that applying the accumulate method to
    /// the info derived from two leaves gives the same result as
    /// deriving the info from the concatenation of the two leaves. In
    /// mathematical terms, the compute_info method is a monoid
    /// homomorphism.
    fn compute_info(&Self::L) -> Self;

    /// The identity of the monoid. Need not be implemented because it
    /// can be computed from the leaf default.
    fn identity() -> Self {
        Self::compute_info(&Self::L::default())
    }

    /// The interval covered by this node. Will generally be implemented
    /// in interval trees; the default impl is sufficient for other types.
    fn interval(&self, len: usize) -> Interval {
        Interval::new_closed_closed(0, len)
    }
}

pub trait Leaf: Sized + Clone + Default {

    // measurement of leaf in base units
    fn len(&self) -> usize;

    // generally a minimum size requirement for leaves
    fn is_ok_child(&self) -> bool;

    // Interval is in "base units"
    // generally implements a maximum size
    // Invariant: if one or the other input is empty, then no split

    // Invariant: if either input satisfies is_ok_child, then on return self
    // satisfies this, as does the optional split.

    fn push_maybe_split(&mut self, other: &Self, iv: Interval) -> Option<Self>;

    // same meaning as push_maybe_split starting from an empty
    // leaf, but maybe can be implemented more efficiently?
    // TODO: remove if it doesn't pull its weight
    fn subseq(&self, iv: Interval) -> Self {
        let mut result = Self::default();
        if result.push_maybe_split(self, iv).is_some() {
            panic!("unexpected split");
        }
        result
    }
}

/// A b-tree node storing leaves at the bottom, and with info
/// retained at each node. It is implemented with atomic reference counting
/// and copy-on-write semantics, so an immutable clone is a very cheap
/// operation, and nodes can be shared across threads. Even so, it is
/// designed to be updated in place, with efficiency similar to a mutable
/// data structure, using uniqueness of reference count to detect when
/// this operation is safe.
///
/// When the leaf is a string, this is a rope data structure (a persistent
/// rope in functional programming jargon). However, it is not restricted
/// to strings, and it is expected to be the basis for a number of data
/// structures useful for text processing.
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
    fn measure(&N, usize) -> usize;

    fn to_base_units(l: &N::L, in_measured_units: usize) -> usize;

    fn from_base_units(l: &N::L, in_base_units: usize) -> usize;

    // The next three methods work in base units.

    // These methods must indicate a boundary at the end of a leaf,
    // if present. A boundary at the beginning of a leaf is optional
    // (the previous leaf will be queried).

    fn is_boundary(l: &N::L, offset: usize) -> bool;

    // will be called with offset > 0
    fn prev(l: &N::L, offset: usize) -> Option<usize>;

    fn next(l: &N::L, offset: usize) -> Option<usize>;

    // When can_fragment is false, the ends of leaves are always
    // considered to be boundaries. More formally:
    // !can_fragment -> to_base_units(measure) = leaf.len
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

    fn is_leaf(&self) -> bool {
        self.0.height == 0
    }

    fn interval(&self) -> Interval {
        self.0.info.interval(self.0.len)
    }

    fn get_children(&self) -> &[Node<N>] {
        if let NodeVal::Internal(ref v) = self.0.val {
            v
        } else {
            panic!("get_children called on leaf node");
        }
    }

    fn get_leaf(&self) -> &N::L {
        if let NodeVal::Leaf(ref l) = self.0.val {
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

    fn merge_leaves(mut rope1: Node<N>, rope2: Node<N>) -> Node<N> {
        debug_assert!(rope1.is_leaf() && rope2.is_leaf());

        let both_ok = rope1.get_leaf().is_ok_child() && rope2.get_leaf().is_ok_child();
        if both_ok {
            return Node::from_nodes(vec![rope1, rope2]);
        }
        match {
            let mut node1 = Arc::make_mut(&mut rope1.0);
            let leaf2 = rope2.get_leaf();
            if let NodeVal::Leaf(ref mut leaf1) = node1.val {
                let leaf2_iv = Interval::new_closed_closed(0, leaf2.len());
                let new = leaf1.push_maybe_split(leaf2, leaf2_iv);
                node1.len = leaf1.len();
                node1.info = N::compute_info(leaf1);
                new
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
            None => {
                rope1
            }
        }
    }

    pub fn concat(rope1: Node<N>, rope2: Node<N>) -> Node<N> {
        use std::cmp::Ordering;

        let h1 = rope1.height();
        let h2 = rope2.height();

        match h1.cmp(&h2) {
            Ordering::Less => {
                let children2 = rope2.get_children();
                if h1 == h2 - 1 && rope1.is_ok_child() {
                    return Node::merge_nodes(&[rope1], children2);
                }
                let newrope = Node::concat(rope1, children2[0].clone());
                if newrope.height() == h2 - 1 {
                    Node::merge_nodes(&[newrope], &children2[1..])
                } else {
                    Node::merge_nodes(newrope.get_children(), &children2[1..])
                }
            },
            Ordering::Equal => {
                if rope1.is_ok_child() && rope2.is_ok_child() {
                    return Node::from_nodes(vec![rope1, rope2]);
                }
                if h1 == 0 {
                    return Node::merge_leaves(rope1, rope2);
                }
                Node::merge_nodes(rope1.get_children(), rope2.get_children())
            },
            Ordering::Greater => {
                let children1 = rope1.get_children();
                if h2 == h1 - 1 && rope2.is_ok_child() {
                    return Node::merge_nodes(children1, &[rope2]);
                }
                let lastix = children1.len() - 1;
                let newrope = Node::concat(children1[lastix].clone(), rope2);
                if newrope.height() == h1 - 1 {
                    Node::merge_nodes(&children1[..lastix], &[newrope])
                } else {
                    Node::merge_nodes(&children1[..lastix], newrope.get_children())
                }
            }
        }
    }

    pub fn measure<M: Metric<N>>(&self) -> usize {
        M::measure(&self.0.info, self.0.len)
    }

    /*
    // TODO: not sure if this belongs in the public interface, cursor
    // might subsume all real use cases.
    // calls the given function with leaves forming the sequence
    fn visit_subseq<F>(&self, iv: Interval, f: &mut F)
            where F: FnMut(&N::L) -> () {
        if iv.is_empty() {
            return;
        }
        match self.0.val {
            NodeVal::Leaf(ref l) => {
                if iv == Interval::new_closed_closed(0, l.len()) {
                    f(l);
                } else {
                    f(&l.clone().subseq(iv));
                }
            }
            NodeVal::Internal(ref v) => {
                let mut offset = 0;
                for child in v {
                    if iv.is_before(offset) {
                        break;
                    }
                    let child_iv = Interval::new_closed_closed(0, child.len());
                    // easier just to use signed ints?
                    let rec_iv = iv.intersect(child_iv.translate(offset))
                        .translate_neg(offset);
                    child.visit_subseq::<F>(rec_iv, f);
                    offset += child_iv.size();
                }
                return;
            }
        }
    }
    */

    pub fn push_subseq(&self, b: &mut TreeBuilder<N>, iv: Interval) {
        if iv.is_empty() {
            return;
        }
        if iv == self.interval() {
            b.push(self.clone());
            return;
        }
        match self.0.val {
            NodeVal::Leaf(ref l) => {
                b.push_leaf_slice(l, iv);
            }
            NodeVal::Internal(ref v) => {
                let mut offset = 0;
                for child in v {
                    if iv.is_before(offset) {
                        break;
                    }
                    let child_iv = child.interval();
                    // easier just to use signed ints?
                    let rec_iv = iv.intersect(child_iv.translate(offset))
                        .translate_neg(offset);
                    child.push_subseq(b, rec_iv);
                    offset += child.len();
                }
                return;
            }
        }
    }

    pub fn subseq(&self, iv: Interval) -> Node<N> {
        let mut b = TreeBuilder::new();
        self.push_subseq(&mut b, iv);
        b.build()
    }

    pub fn edit(&mut self, iv: Interval, new: Node<N>) {
        let mut b = TreeBuilder::new();
        let self_iv = Interval::new_closed_closed(0, self.len());
        self.push_subseq(&mut b, self_iv.prefix(iv));
        b.push(new);
        self.push_subseq(&mut b, self_iv.suffix(iv));
        *self = b.build();
    }

    // doesn't deal with endpoint, handle that specially if you need it
    pub fn convert_metrics<M1: Metric<N>, M2: Metric<N>>(&self, mut m1: usize) -> usize {
        if m1 == 0 { return 0; }
        // If M1 can fragment, then we must land on the leaf containing
        // the m1 boundary. Otherwise, we can land on the beginning of
        // the leaf immediately following the M1 boundary, which may be
        // more efficient.
        let m1_fudge = if M1::can_fragment() { 1 } else { 0 };
        let mut m2 = 0;
        let mut node = self;
        while node.height() > 0 {
            for child in node.get_children() {
                let child_m1 = child.measure::<M1>();
                if m1 < child_m1 + m1_fudge {
                    node = child;
                    break;
                }
                m2 += child.measure::<M2>();
                m1 -= child_m1;
            }
        }
        let l = node.get_leaf();
        let base = M1::to_base_units(l, m1);
        m2 + M2::from_base_units(l, base)
    }
}

impl<N: NodeInfo> Default for Node<N> {
    fn default() -> Node<N> {
        Node::from_leaf(N::L::default())
    }
}

pub struct TreeBuilder<N: NodeInfo>(Option<Node<N>>);

impl<N: NodeInfo> TreeBuilder<N> {
    pub fn new() -> TreeBuilder<N> {
        TreeBuilder(None)
    }

    // TODO: more sophisticated implementation, so pushing a sequence
    // is amortized O(n), rather than O(n log n) as now.
    pub fn push(&mut self, n: Node<N>) {
        match self.0.take() {
            None => self.0 = Some(n),
            Some(buf) => self.0 = Some(Node::concat(buf, n))
        }
    }

    pub fn push_leaf(&mut self, l: N::L) {
        self.push(Node::from_leaf(l))
    }

    pub fn push_leaf_slice(&mut self, l: &N::L, iv: Interval) {
        self.push(Node::from_leaf(l.subseq(iv)))
    }

    pub fn build(self) -> Node<N> {
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
    // invariant: offset is at end of leaf iff end of rope
    pub fn get_leaf(&self) -> Option<(&'a N::L, usize)> {
        self.leaf.map(|l| (l, self.position - self.offset_of_leaf))
    }

    pub fn set(&mut self, position: usize) {
        self.position = position;
        if let Some(l) = self.leaf {
            if self.position >= self.offset_of_leaf &&
                    self.position < self.offset_of_leaf + l.len() {
                return;
            }
        }
        // TODO: walk up tree to find leaf if nearby
        self.descend();
    }

    pub fn pos(&self) -> usize {
        self.position
    }

    pub fn is_boundary<M: Metric<N>>(&mut self) -> bool {
        if self.leaf.is_none() {
            // not at a valid position
            return false;
        }
        if self.position == 0 ||
                (self.position == self.offset_of_leaf && !M::can_fragment()) {
            return true;
        }
        if self.position > self.offset_of_leaf {
            return M::is_boundary(self.leaf.unwrap(),
                self.position - self.offset_of_leaf);
        }
        // tricky case, at beginning of leaf, need to query end of previous
        // leaf; TODO: would be nice if we could do it another way that didn't
        // make the method &self mut.
        let l = self.prev_leaf().unwrap().0;
        let result = M::is_boundary(l, l.len());
        let _ = self.next_leaf();
        result
    }

    pub fn prev<M: Metric<N>>(&mut self) -> Option<(usize)> {
        if self.position == 0 || self.leaf.is_none() {
            self.leaf = None;
            return None;
        }
        let orig_pos = self.position;
        let offset_in_leaf = orig_pos - self.offset_of_leaf;
        if let Some(l) = self.leaf {
            if offset_in_leaf > 0 {
                if let Some(offset_in_leaf) = M::prev(l, offset_in_leaf) {
                    self.position = self.offset_of_leaf + offset_in_leaf;
                    return Some(self.position);
                }
            }
        } else {
            panic!("inconsistent, shouldn't get here");
        }
        // not in same leaf, need to scan backwards
        // TODO: walk up tree to skip measure-0 nodes
        loop {
            if self.offset_of_leaf == 0 {
                self.position = 0;
                return Some(self.position);
            }
            if let Some((l, _)) = self.prev_leaf() {
                // TODO: node already has this, no need to recompute. But, we
                // should be looking at nodes anyway at this point, as we need
                // to walk up the tree.
                let node_info = N::compute_info(l);
                if M::measure(&node_info, l.len()) == 0 {
                    // leaf doesn't contain boundary, keep scanning
                    continue;
                }
                if self.offset_of_leaf + l.len() < orig_pos && M::is_boundary(l, l.len()) {
                    let _ = self.next_leaf();
                    return Some(self.position);
                }
                if let Some(offset_in_leaf) = M::prev(l, l.len()) {
                    self.position = self.offset_of_leaf + offset_in_leaf;
                    return Some(self.position);
                } else {
                    panic!("metric is inconsistent, metric > 0 but no boundary");
                }
            }
        }
    }

    pub fn next<M: Metric<N>>(&mut self) -> Option<(usize)> {
        if self.position >= self.root.len() || self.leaf.is_none() {
            self.leaf = None;
            return None;
        }
        // TODO: walk up tree to skip measure-0 nodes
        loop {
            if let Some(l) = self.leaf {
                let offset_in_leaf = self.position - self.offset_of_leaf;
                if let Some(offset_in_leaf) = M::next(l, offset_in_leaf) {
                    if offset_in_leaf == l.len() &&
                            self.offset_of_leaf + offset_in_leaf != self.root.len() {
                        let _ = self.next_leaf();
                    } else {
                        self.position = self.offset_of_leaf + offset_in_leaf;
                    }
                    return Some(self.position);
                }
                if self.offset_of_leaf + l.len() == self.root.len() {
                    self.position = self.root.len();
                    return Some(self.position);
                }
                let _ = self.next_leaf();
            } else {
                panic!("inconsistent, shouldn't get here");
            }
        }
    }

    // same return as get_leaf, moves to beginning of next leaf
    pub fn next_leaf(&mut self) -> Option<(&'a N::L, usize)> {
        if let Some(leaf) = self.leaf {
            self.position = self.offset_of_leaf + leaf.len();
        } else {
            return None;
        }
        for i in 0..CURSOR_CACHE_SIZE {
            if self.cache[i].is_none() {
                // this probably can't happen
                self.leaf = None;
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
        if self.offset_of_leaf + self.leaf.unwrap().len() == self.root.len() {
            self.leaf = None;
            return None;
        }
        self.descend();
        self.get_leaf()
    }

    // same return as get_leaf, moves to beginning of prev leaf
    pub fn prev_leaf(&mut self) -> Option<(&'a N::L, usize)> {
        if self.offset_of_leaf == 0 || Some(self.leaf).is_none() {
            return None;
        }
        for i in 0..CURSOR_CACHE_SIZE {
            if self.cache[i].is_none() {
                self.leaf = None;
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
                if i + 1 == children.len() {
                    break;
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

/*
// TODO: the following is an example, written during development but
// not actually used. Either make it real or delete it.

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

    fn push_maybe_split(&mut self, other: &Vec<u8>, iv: Interval) -> Option<Vec<u8>> {
        let (start, end) = iv.start_end();
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
    type BaseMetric = BytesMetric;
    fn accumulate(&mut self, other: &Self) {
        self.0 += other.0;
    }

    fn compute_info(l: &Vec<u8>) -> BytesInfo {
        BytesInfo(l.len())
    }
}

struct BytesMetric(());

impl Metric<BytesInfo> for BytesMetric {
    fn measure(_: &BytesInfo, len: usize) -> usize {
        len
    }

    fn to_base_units(_: &Vec<u8>, in_measured_units: usize) -> usize {
        in_measured_units
    }

    fn from_base_units(_: &Vec<u8>, in_base_units: usize) -> usize {
        in_base_units
    }

    fn is_boundary(_: &Vec<u8>, _: usize) -> bool { true }

    fn prev(_: &Vec<u8>, offset: usize) -> Option<usize> {
        if offset > 0 { Some(offset - 1) } else { None }
    }

    fn next(l: &Vec<u8>, offset: usize) -> Option<usize> {
        if offset < l.len() { Some(offset + 1) } else { None }
    }

    fn can_fragment() -> bool { false }
}

*/
