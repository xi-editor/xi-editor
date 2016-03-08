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

#[derive(Clone, Debug)]
pub struct Rope(Rc<RopeNode>);

#[derive(Debug)]
struct RopeNode {
    height: usize,
    size: usize,
    val: RopeVal
}

#[derive(Debug)]
enum RopeVal {
    Leaf(String),
    Internal(Vec<Rope>)
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
        let mut b = RopeBuilder::new();
        b.push_str(s);
        b.build()
    }

    fn new(n: RopeNode) -> Rope {
        Rope(Rc::new(n))
    }

    fn height(&self) -> usize {
        self.0.height
    }

    // rename to len, to be consistent with Rust?
    pub fn size(&self) -> usize {
        self.0.size
    }

    fn get_children(&self) -> &[Rope] {
        if let &RopeVal::Internal(ref v) = &self.0.val {
            v
        } else {
            panic!("get_children called on leaf node");
        }
    }

    fn is_ok_child(&self) -> bool {
        match &self.0.val {
            &RopeVal::Leaf(_) => (self.size() >= MIN_LEAF),
            &RopeVal::Internal(ref pieces) => (pieces.len() >= MIN_CHILDREN)
        }
    }

    // precondition: s.len() <= MAX_LEAF
    fn from_string_piece(s: String) -> Rope {
        Rope::new(RopeNode {
            height: 0,
            size: s.len(),
            val: RopeVal::Leaf(s)
        })
    }

    // precondition 2 <= pieces.len() <= MAX_CHILDREN
    fn from_pieces(pieces: Vec<Rope>) -> Rope {
        Rope::new(RopeNode {
            height: pieces[0].height() + 1,
            size: pieces.iter().fold(0, |sum, r| sum + r.size()),
            val: RopeVal::Internal(pieces)
        })
    }

    fn merge_nodes(children1: &[Rope], children2: &[Rope]) -> Rope {
        let n_children = children1.len() + children2.len();
        if n_children <= MAX_CHILDREN {
            let mut v = Vec::with_capacity(n_children);
            v.extend_from_slice(children1);
            v.extend_from_slice(children2);
            Rope::from_pieces(v)
        } else {
            // Note: this leans left. Splitting at midpoint is also an option
            let splitpoint = min(MAX_CHILDREN, n_children - MIN_CHILDREN);
            let mut iter = children1.iter().chain(children2.iter()).cloned();
            let left = iter.by_ref().take(splitpoint).collect();
            let right = iter.collect();
            let parent_pieces = vec![Rope::from_pieces(left), Rope::from_pieces(right)];
            Rope::from_pieces(parent_pieces)
        }
    }

    // precondition: both ropes are leaves
    fn merge_leaves(rope1: Rope, rope2: Rope) -> Rope {
        if rope1.size() >= MIN_LEAF && rope2.size() >= MIN_LEAF {
            return Rope::from_pieces(vec![rope1, rope2]);
        }
        // TODO: try to reuse rope1 if uniquely owned
        match (&rope1.0.val, &rope2.0.val) {
            (&RopeVal::Leaf(ref s1), &RopeVal::Leaf(ref s2)) => {
                let size = s1.len() + s2.len();
                // There might be a more convenient idiom for this, but it
                // guarantees the desired allocation behavior.
                let mut s = String::with_capacity(size);
                s.push_str(s1);
                s.push_str(s2);
                if size <= MAX_LEAF {
                    Rope::from_string_piece(s)
                } else {
                    let splitpoint = find_leaf_split_for_merge(&s);
                    let right_str = s[splitpoint..].to_string();
                    s.truncate(splitpoint);
                    // TODO: s probably has too much capacity, is wasteful
                    let left = Rope::from_string_piece(s);
                    let right = Rope::from_string_piece(right_str);
                    Rope::from_pieces(vec![left, right])
                }
            },
            _ => panic!("merge_leaves called with non-leaf node")
        }
    }

    pub fn concat(rope1: Rope, rope2: Rope) -> Rope {
        let h1 = rope1.height();
        let h2 = rope2.height();
        if h1 == h2 {
            if rope1.is_ok_child() && rope2.is_ok_child() {
                return Rope::from_pieces(vec![rope1, rope2]);
            }
            if h1 == 0 {
                return Rope::merge_leaves(rope1, rope2);
            }
            return Rope::merge_nodes(rope1.get_children(), rope2.get_children());
        } else if h1 < h2 {
            let children2 = rope2.get_children();
            if h1 == h2 - 1 && rope1.is_ok_child() {
                return Rope::merge_nodes(&[rope1], children2);
            }
            let newrope = Rope::concat(rope1, children2[0].clone());
            if newrope.height() == h2 - 1 {
                return Rope::merge_nodes(&[newrope], &children2[1..]);
            } else {
                return Rope::merge_nodes(newrope.get_children(), &children2[1..]);
            }
        } else {  // h1 > h2
            let children1 = rope1.get_children();
            if h2 == h1 - 1 && rope2.is_ok_child() {
                return Rope::merge_nodes(children1, &[rope2]);
            }
            let lastix = children1.len() - 1;
            let newrope = Rope::concat(children1[lastix].clone(), rope2);
            if newrope.height() == h1 - 1 {
                return Rope::merge_nodes(&children1[..lastix], &[newrope]);
            } else {
                return Rope::merge_nodes(&children1[..lastix], newrope.get_children());
            }
        }
    }

    fn subsequence_rec(&self, b: &mut RopeBuilder, start: usize, end: usize) {
        if start == 0 && self.size() == end {
            b.push(self.clone());
            return
        }
        match &self.0.val {
            &RopeVal::Leaf(ref s) => b.push_str_short(&s[start..end]),
            &RopeVal::Internal(ref v) => {
                let mut offset = 0;
                for child in v {
                    if end <= offset {
                        break;
                    }
                    if offset + child.size() > start {
                        //println!("start={}, end={}, offset={}, child.size={}", start, end, offset, child.size());
                        child.subsequence_rec(b, max(offset, start) - offset, min(child.size(), end - offset))
                    }
                    offset += child.size()
                }
            }
        }
    }

    pub fn subrange(self, start: usize, end: usize) -> Rope {
        let mut b = RopeBuilder::new();
        self.subsequence_rec(&mut b, start, end);
        b.build()
    }

    pub fn replace(self, start: usize, end: usize, new: Rope) -> Rope {
        if let &RopeVal::Leaf(ref s) = &new.0.val {
            if s.len() < MIN_LEAF {
                return self.replace_str(start, end, s);
            }
        }
        let mut b = RopeBuilder::new();
        self.subsequence_rec(&mut b, 0, start);
        b.push(new);
        self.subsequence_rec(&mut b, end, self.size());
        b.build()
    }

    pub fn replace_str(mut self, start: usize, end: usize, new: &str) -> Rope {
        if new.len() < MIN_LEAF {
            // try to do replacement without changing tree structure
            if Rope::try_replace_str(&mut self, start, end, new) {
                return self;
            }
        }
        let mut b = RopeBuilder::new();
        self.subsequence_rec(&mut b, 0, start);
        b.push_str(new);
        self.subsequence_rec(&mut b, end, self.size());
        b.build()
    }

    // precondition: leaf
    fn try_replace_leaf_str(r: &mut Rope, start: usize, end: usize, new: &str) -> bool {
        // TODO: maybe try to mutate in place, using either unsafe or
        // special-case single-char insert and remove (plus trunc, append)

        let size_plus_new = r.size() + new.len();
        if size_plus_new < MIN_LEAF + (end - start) || size_plus_new > MAX_LEAF + (end - start) {
            return false;
        }
        let newsize = size_plus_new - (end - start);
        *r = {
            if let &RopeVal::Leaf(ref s) = &r.0.val {
                let mut newstr = String::with_capacity(newsize);
                newstr.push_str(&s[..start]);
                newstr.push_str(new);
                newstr.push_str(&s[end..]);
                Rope::from_string_piece(newstr)
            } else {
                panic!("height and node type inconsistent");
            }
        };
        return true;
    }

    // return child index and offset on success
    fn try_find_child(children: &[Rope], start: usize, end: usize) -> Option<(usize, usize)> {
        // TODO: maybe try scanning from back if close to end (would need parent's size)
        let mut offset = 0;
        let mut i = 0;
        while i < children.len() {
            let nextoff = offset + children[i].size();
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

    fn try_replace_str(r: &mut Rope, start: usize, end: usize, new: &str) -> bool {
        if r.height() == 0 {
            return Rope::try_replace_leaf_str(r, start, end, new);
        }
        if let Some(node) = Rc::get_mut(&mut r.0) {
            // unique reference, let's try to mutate in place
            let mut success = false;
            match node.val {
                RopeVal::Internal(ref mut v) => {
                    if let Some((i, offset)) = Rope::try_find_child(&v, start, end) {
                        success = Rope::try_replace_str(&mut v[i], start - offset, end - offset, new);
                        if success {
                            // update invariants
                            node.size = node.size - (end - start) + new.len();
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
            let children = r.get_children();
            if let Some((i, offset)) = Rope::try_find_child(children, start, end) {
                let mut child = children[i].clone();
                if Rope::try_replace_str(&mut child, start - offset, end - offset, new) {
                    let mut v = Vec::with_capacity(children.len());
                    v.extend_from_slice(&children[..i]);
                    v.push(child);
                    v.extend_from_slice(&children[i + 1 ..]);
                    result = Some(Rope::from_pieces(v));
                }
            }
        }
        if let Some(newrope) = result {
            *r = newrope;
            return true;
        }
        false
    }

    fn push_to_string(&self, dst: &mut String) {
        match &self.0.val {
            &RopeVal::Leaf(ref s) => dst.push_str(&s),
            &RopeVal::Internal(ref v) => {
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
                        RopeVal::Leaf(s) => return s,
                        _ => panic!("height and node type inconsistent")
                    }
                },
                Err(r) => Rope(r)
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
struct RopeBuilder(Option<Rope>);

impl RopeBuilder {
    fn new() -> RopeBuilder {
        RopeBuilder(None)
    }

    fn push(&mut self, r: Rope) {
        match self.0.take() {
            None => self.0 = Some(r),
            Some(buf) => self.0 = Some(Rope::concat(buf, r))
        }
    }

    // precondition: s.len() <= MAX_LEAF
    fn push_str_short(&mut self, s: &str) {
        self.push(Rope::from_string_piece(s.to_string()));
    }

    fn push_str(&mut self, mut s: &str) {
        if s.len() <= MAX_LEAF {
            if s.len() > 0 {
                self.push_str_short(s);
            }
            return;
        }
        let mut stack: Vec<Vec<Rope>> = Vec::new();
        while s.len() > 0 {
            let splitpoint = if s.len() > MAX_LEAF {
                find_leaf_split_for_bulk(s)
            } else {
                s.len()
            };
            let mut new = Rope::from_string_piece(s[..splitpoint].to_string());
            s = &s[splitpoint..];
            loop {
                if stack.last().map_or(true, |r| r[0].height() != new.height()) {
                    stack.push(Vec::new());
                }
                stack.last_mut().unwrap().push(new);
                if stack.last().unwrap().len() < MAX_CHILDREN {
                    break;
                }
                new = Rope::from_pieces(stack.pop().unwrap())
            }
        }
        for v in stack {
            for r in v {
                self.push(r)
            }
        }
    }

    fn build(self) -> Rope {
        match self.0 {
            None => Rope::from_string_piece(String::new()),
            Some(buf) => buf
        }
    }
}

#[test]
fn concat_small() {
    let a = Rope::from_str("hello ");
    let b = Rope::from_str("world");
    assert_eq!("hello world", Rope::concat(a, b).into_string());
}

#[test]
fn subrange_small() {
    let a = Rope::from_str("hello world");
    assert_eq!("ello wor", a.subrange(1, 9).into_string());
}

#[test]
fn replace_small() {
    let a = Rope::from_str("hello world");
    assert_eq!("herald", a.replace_str(1, 9, "era").into_string());
}

#[test]
fn append_large() {
    let mut a = Rope::from_str("");
    let mut b = String::new();
    for i in 0..100_000 {
        let c = i.to_string() + "\n";
        let l = a.size();
        a = a.replace_str(l, l, &c);
        b.push_str(&c);
    }
    assert_eq!(b, a.into_string());
}
