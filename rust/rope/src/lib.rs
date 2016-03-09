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

//! A rope data structure suitable for text editing

use std::rc::Rc;
use std::cmp::{min,max};
use std::ops::Add;
//use std::fmt::{Debug, Formatter};

const MIN_LEAF: usize = 511;
const MAX_LEAF: usize = 1024;
const MIN_CHILDREN: usize = 4;
const MAX_CHILDREN: usize = 8;

// TODO: probably will be stabilized in Rust std lib
// Note, this isn't exactly the same, it panics when index > s.len()
fn is_char_boundary(s: &str, index: usize) -> bool {
    index == s.len() || (s.as_bytes()[index] & 0xc0) != 0x80
}

// The main rope data structure.
pub struct Rope {
    root: Node,
    start: usize,
    len: usize
}

#[derive(Clone, Debug)]
struct Node(Rc<NodeBody>);

#[derive(Debug)]
struct NodeBody {
    height: usize,
    len: usize,
    val: NodeVal
}

#[derive(Debug)]
enum NodeVal {
    Leaf(String),
    Internal(Vec<Node>)
}

/*
impl Debug for Rope {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        write!(f, "Rope[height={}, size={}]", self.0.height, self.size())
    }
}
*/

impl Rope {
    pub fn from_str(s: &str) -> Rope {
        Rope::from_node(Node::from_str(s))
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn into_string(self) -> String {
        // TODO: normalize call can be wasteful
        self.normalize().root.into_string()
    }

    // Maybe take Range arguments as well (would need traits)
    pub fn slice(self, mut start: usize, mut end: usize) -> Rope {
        let mut root = self.root;
        while root.height() > 0 {
            if let Some((i, offset)) = Node::try_find_child(root.get_children(), start, end) {
                root = root.get_children()[i].clone();
                start -= offset;
                end -= offset;
            } else {
                break;
            }
        }
        Rope {
            root: root,
            start: start + self.start,
            len: end - start
        }
    }

    pub fn edit(self, start: usize, end: usize, new: Rope) -> Rope {
        if self.is_full() {
            return Rope::from_node(self.root.replace(start, end, new.normalize().root));
        }
        let mut b = RopeBuilder::new();
        self.root.subsequence_rec(&mut b, self.start, self.start + start);
        b.push_rope(new);
        self.root.subsequence_rec(&mut b, self.start + end, self.start + self.len);
        b.build_rope()
    }

    pub fn edit_str(self, start: usize, end: usize, new: &str) -> Rope {
        if self.is_full() {
            return Rope::from_node(self.root.replace_str(start, end, new));
        }
        let mut b = RopeBuilder::new();
        self.root.subsequence_rec(&mut b, self.start, self.start + start);
        b.push_str(new);
        self.root.subsequence_rec(&mut b, self.start + end, self.start + self.len);
        b.build_rope()
    }

    // return condition: result is_full
    fn normalize(self) -> Rope {
        if self.is_full() {
            self
        } else {
            let mut b = RopeBuilder::new();
            b.push_rope(self);
            b.build_rope()
        }
    }

    fn is_full(&self) -> bool {
        self.start == 0 && self.len == self.root.len()
    }

    fn from_node(n: Node) -> Rope {
        let len = n.len();
        Rope {
            root: n,
            start: 0,
            len: len
        }
    }
}

impl Add<Rope> for Rope {
    type Output = Rope;
    fn add(self, rhs: Rope) -> Rope {
        let mut b = RopeBuilder::new();
        b.push_rope(self);
        b.push_rope(rhs);
        b.build_rope()
    }
}

// Not sure I want to commit to this, it shadows Add<String>, which might be optimized
// to reuse the string's allocation.
impl<T: AsRef<str>> Add<T> for Rope {
    type Output = Rope;
    fn add(self, rhs: T) -> Rope {
        let lhs = self.normalize();
        let len = lhs.len();
        Rope::from_node(lhs.root.replace_str(len, len, rhs.as_ref()))
    }
}

impl Node {
    fn from_str(s: &str) -> Node {
        let mut b = RopeBuilder::new();
        b.push_str(s);
        b.build()
    }

    fn new(n: NodeBody) -> Node {
        Node(Rc::new(n))
    }

    fn height(&self) -> usize {
        self.0.height
    }

    // rename to len, to be consistent with Rust?
    pub fn len(&self) -> usize {
        self.0.len
    }

    fn get_children(&self) -> &[Node] {
        if let &NodeVal::Internal(ref v) = &self.0.val {
            v
        } else {
            panic!("get_children called on leaf node");
        }
    }

    fn is_ok_child(&self) -> bool {
        match &self.0.val {
            &NodeVal::Leaf(_) => (self.len() >= MIN_LEAF),
            &NodeVal::Internal(ref pieces) => (pieces.len() >= MIN_CHILDREN)
        }
    }

    // precondition: s.len() <= MAX_LEAF
    fn from_string_piece(s: String) -> Node {
        Node::new(NodeBody {
            height: 0,
            len: s.len(),
            val: NodeVal::Leaf(s)
        })
    }

    // precondition 2 <= pieces.len() <= MAX_CHILDREN
    fn from_pieces(pieces: Vec<Node>) -> Node {
        Node::new(NodeBody {
            height: pieces[0].height() + 1,
            len: pieces.iter().fold(0, |sum, r| sum + r.len()),
            val: NodeVal::Internal(pieces)
        })
    }

    fn merge_nodes(children1: &[Node], children2: &[Node]) -> Node {
        let n_children = children1.len() + children2.len();
        if n_children <= MAX_CHILDREN {
            let mut v = Vec::with_capacity(n_children);
            v.extend_from_slice(children1);
            v.extend_from_slice(children2);
            Node::from_pieces(v)
        } else {
            // Note: this leans left. Splitting at midpoint is also an option
            let splitpoint = min(MAX_CHILDREN, n_children - MIN_CHILDREN);
            let mut iter = children1.iter().chain(children2.iter()).cloned();
            let left = iter.by_ref().take(splitpoint).collect();
            let right = iter.collect();
            let parent_pieces = vec![Node::from_pieces(left), Node::from_pieces(right)];
            Node::from_pieces(parent_pieces)
        }
    }

    // precondition: both ropes are leaves
    fn merge_leaves(rope1: Node, rope2: Node) -> Node {
        if rope1.len() >= MIN_LEAF && rope2.len() >= MIN_LEAF {
            return Node::from_pieces(vec![rope1, rope2]);
        }
        // TODO: try to reuse rope1 if uniquely owned
        match (&rope1.0.val, &rope2.0.val) {
            (&NodeVal::Leaf(ref s1), &NodeVal::Leaf(ref s2)) => {
                let size = s1.len() + s2.len();
                // There might be a more convenient idiom for this, but it
                // guarantees the desired allocation behavior.
                let mut s = String::with_capacity(size);
                s.push_str(s1);
                s.push_str(s2);
                if size <= MAX_LEAF {
                    Node::from_string_piece(s)
                } else {
                    let splitpoint = find_leaf_split_for_merge(&s);
                    let right_str = s[splitpoint..].to_string();
                    s.truncate(splitpoint);
                    // TODO: s probably has too much capacity, is wasteful
                    let left = Node::from_string_piece(s);
                    let right = Node::from_string_piece(right_str);
                    Node::from_pieces(vec![left, right])
                }
            },
            _ => panic!("merge_leaves called with non-leaf node")
        }
    }

    fn concat(rope1: Node, rope2: Node) -> Node {
        let h1 = rope1.height();
        let h2 = rope2.height();
        if h1 == h2 {
            if rope1.is_ok_child() && rope2.is_ok_child() {
                return Node::from_pieces(vec![rope1, rope2]);
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

    fn subsequence_rec(&self, b: &mut RopeBuilder, start: usize, end: usize) {
        if start == 0 && self.len() == end {
            b.push(self.clone());
            return
        }
        match &self.0.val {
            &NodeVal::Leaf(ref s) => b.push_str_short(&s[start..end]),
            &NodeVal::Internal(ref v) => {
                let mut offset = 0;
                for child in v {
                    if end <= offset {
                        break;
                    }
                    if offset + child.len() > start {
                        //println!("start={}, end={}, offset={}, child.size={}", start, end, offset, child.size());
                        child.subsequence_rec(b, max(offset, start) - offset, min(child.len(), end - offset))
                    }
                    offset += child.len()
                }
            }
        }
    }

    fn replace(self, start: usize, end: usize, new: Node) -> Node {
        if let &NodeVal::Leaf(ref s) = &new.0.val {
            if s.len() < MIN_LEAF {
                return self.replace_str(start, end, s);
            }
        }
        let mut b = RopeBuilder::new();
        self.subsequence_rec(&mut b, 0, start);
        b.push(new);
        self.subsequence_rec(&mut b, end, self.len());
        b.build()
    }

    fn replace_str(mut self, start: usize, end: usize, new: &str) -> Node {
        if new.len() < MIN_LEAF {
            // try to do replacement without changing tree structure
            if Node::try_replace_str(&mut self, start, end, new) {
                return self;
            }
        }
        let mut b = RopeBuilder::new();
        self.subsequence_rec(&mut b, 0, start);
        b.push_str(new);
        self.subsequence_rec(&mut b, end, self.len());
        b.build()
    }

    // precondition: leaf
    fn try_replace_leaf_str(n: &mut Node, start: usize, end: usize, new: &str) -> bool {
        // TODO: maybe try to mutate in place, using either unsafe or
        // special-case single-char insert and remove (plus trunc, append)

        let size_plus_new = n.len() + new.len();
        if size_plus_new < MIN_LEAF + (end - start) || size_plus_new > MAX_LEAF + (end - start) {
            return false;
        }
        let newsize = size_plus_new - (end - start);
        *n = {
            if let &NodeVal::Leaf(ref s) = &n.0.val {
                let mut newstr = String::with_capacity(newsize);
                newstr.push_str(&s[..start]);
                newstr.push_str(new);
                newstr.push_str(&s[end..]);
                Node::from_string_piece(newstr)
            } else {
                panic!("height and node type inconsistent");
            }
        };
        return true;
    }

    // return child index and offset on success
    fn try_find_child(children: &[Node], start: usize, end: usize) -> Option<(usize, usize)> {
        // TODO: maybe try scanning from back if close to end (would need parent's size)
        let mut offset = 0;
        let mut i = 0;
        while i < children.len() {
            let nextoff = offset + children[i].len();
            if nextoff >= start {
                if nextoff <= end {
                    return Some((i, offset));
                } else {
                    return None;
                }
            }
            offset = nextoff;
            i += 1;
        }
        None
    }

    fn try_replace_str(n: &mut Node, start: usize, end: usize, new: &str) -> bool {
        if n.height() == 0 {
            return Node::try_replace_leaf_str(n, start, end, new);
        }
        if let Some(node) = Rc::get_mut(&mut n.0) {
            // unique reference, let's try to mutate in place
            let mut success = false;
            match node.val {
                NodeVal::Internal(ref mut v) => {
                    if let Some((i, offset)) = Node::try_find_child(&v, start, end) {
                        success = Node::try_replace_str(&mut v[i], start - offset, end - offset, new);
                        if success {
                            // update invariants
                            node.len = node.len - (end - start) + new.len();
                        }
                    }
                },
                _ => panic!("height and node type inconsistent")
            }
            return success;
        }

        // can't mutate in place, try recursing and making copy
        let mut result = None;
        {
            let children = n.get_children();
            if let Some((i, offset)) = Node::try_find_child(children, start, end) {
                let mut child = children[i].clone();
                if Node::try_replace_str(&mut child, start - offset, end - offset, new) {
                    let mut v = Vec::with_capacity(children.len());
                    v.extend_from_slice(&children[..i]);
                    v.push(child);
                    v.extend_from_slice(&children[i + 1 ..]);
                    result = Some(Node::from_pieces(v));
                }
            }
        }
        if let Some(newrope) = result {
            *n = newrope;
            return true;
        }
        false
    }

    fn push_to_string(&self, dst: &mut String) {
        match &self.0.val {
            &NodeVal::Leaf(ref s) => dst.push_str(&s),
            &NodeVal::Internal(ref v) => {
                for child in v {
                    child.push_to_string(dst);
                }
            }
        }
    }

    fn into_string(self) -> String {
        let rope = if self.height() == 0 {
            match Rc::try_unwrap(self.0) {
                Ok(node) => {
                    match node.val {
                        NodeVal::Leaf(s) => return s,
                        _ => panic!("height and node type inconsistent")
                    }
                },
                Err(r) => Node(r)
            }
        } else {
            self
        };
        let mut result = String::new();
        rope.push_to_string(&mut result);
        result
    }
}

fn find_leaf_split_for_bulk(s: &str) -> usize {
    find_leaf_split(s, MIN_LEAF)
}

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

// TODO: smarter algorithm?
// good case to make this public
struct RopeBuilder(Option<Node>);

impl RopeBuilder {
    fn new() -> RopeBuilder {
        RopeBuilder(None)
    }

    fn push_rope(&mut self, rope: Rope) {
        rope.root.subsequence_rec(self, rope.start, rope.start + rope.len);
    }

    fn push(&mut self, n: Node) {
        match self.0.take() {
            None => self.0 = Some(n),
            Some(buf) => self.0 = Some(Node::concat(buf, n))
        }
    }

    // precondition: s.len() <= MAX_LEAF
    fn push_str_short(&mut self, s: &str) {
        self.push(Node::from_string_piece(s.to_string()));
    }

    fn push_str(&mut self, mut s: &str) {
        if s.len() <= MAX_LEAF {
            if s.len() > 0 {
                self.push_str_short(s);
            }
            return;
        }
        let mut stack: Vec<Vec<Node>> = Vec::new();
        while s.len() > 0 {
            let splitpoint = if s.len() > MAX_LEAF {
                find_leaf_split_for_bulk(s)
            } else {
                s.len()
            };
            let mut new = Node::from_string_piece(s[..splitpoint].to_string());
            s = &s[splitpoint..];
            loop {
                if stack.last().map_or(true, |r| r[0].height() != new.height()) {
                    stack.push(Vec::new());
                }
                stack.last_mut().unwrap().push(new);
                if stack.last().unwrap().len() < MAX_CHILDREN {
                    break;
                }
                new = Node::from_pieces(stack.pop().unwrap())
            }
        }
        for v in stack {
            for r in v {
                self.push(r)
            }
        }
    }

    fn build(self) -> Node {
        match self.0 {
            None => Node::from_string_piece(String::new()),
            Some(buf) => buf
        }
    }

    fn build_rope(self) -> Rope {
        Rope::from_node(self.build())
    }
}

#[test]
fn concat_small() {
    let a = Rope::from_str("hello ");
    let b = Rope::from_str("world");
    assert_eq!("hello world", (a + b).into_string());
}

#[test]
fn subrange_small() {
    let a = Rope::from_str("hello world");
    assert_eq!("ello wor", a.slice(1, 9).into_string());
}

#[test]
fn replace_small() {
    let a = Rope::from_str("hello world");
    assert_eq!("herald", a.edit_str(1, 9, "era").into_string());
}

#[test]
fn append_large() {
    let mut a = Rope::from_str("");
    let mut b = String::new();
    for i in 0..10_000 {
        let c = i.to_string() + "\n";
        b.push_str(&c);
        a = a + c;
    }
    assert_eq!(b, a.into_string());
}
