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

extern crate bytecount;

pub mod tree;
pub mod breaks;
pub mod interval;
pub mod delta;
pub mod rope;
pub mod spans;
pub mod subset;
pub mod engine;

// TODO: "pub use" the types we want to export publicly

// What follows below is slated for deletion, but not all of it has
// been transferred to the new implementation in mod tree.

use std::rc::Rc;
use std::borrow::Cow;
use std::cmp::{min,max};
use std::iter::once;
use std::ops::Add;
use std::str::FromStr;
use std::string::ParseError;
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

/// A rope data structure.
///
/// A [rope](https://en.wikipedia.org/wiki/Rope_(data_structure)) is a data structure
/// for strings, specialized for incremental editing operations. Most operations
/// (such as insert, delete, substring) are O(log n). This module provides an immutable
/// (also known as [persistent](https://en.wikipedia.org/wiki/Persistent_data_structure))
/// version of Ropes, and if there are many copies of similar strings, the common parts
/// are shared.
///
/// Internally, the implementation uses reference counting (not thread safe, though
/// it would be easy enough to modify to use `Arc` instead of `Rc` if that were
/// required). Mutations are generally copy-on-write, though in-place edits are
/// supported as an optimization when only one reference exists, making the
/// implementation as efficient as a mutable version.
///
/// Also note: in addition to the `From` traits described below, this module
/// implements `From<Rope> for String` and `From<&Rope> for String`, for easy
/// conversions in both directions.
///
/// # Examples
///
/// Create a `Rope` from a `String`:
///
/// ```rust
/// # use xi_rope::Rope;
/// let a = Rope::from("hello ");
/// let b = Rope::from("world");
/// assert_eq!("hello world", String::from(a.clone() + b.clone()));
/// assert!("hello world" == a + b);
/// ```
///
/// Get a slice of a `Rope`:
///
/// ```rust
/// # use xi_rope::Rope;
/// let a = Rope::from("hello world");
/// let b = a.slice(1, 9);
/// assert_eq!("ello wor", String::from(&b));
/// let c = b.slice(1, 7);
/// assert_eq!("llo wo", String::from(c));
/// ```
///
/// Replace part of a `Rope`:
///
/// ```rust
/// # use xi_rope::Rope;
/// let mut a = Rope::from("hello world");
/// a.edit_str(1, 9, "era");
/// assert_eq!("herald", String::from(a));
/// ```
#[derive(Clone)]
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
    newline_count: usize,
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

fn count_newlines(s: &str) -> usize {
    bytecount::count(s.as_bytes(), b'\n')
}

impl Rope {
    /// Returns the length of `self`.
    ///
    /// The length is in bytes, the same as `str::len()`.
    ///
    /// Time complexity: O(1)
    pub fn len(&self) -> usize {
        self.len
    }

    /// Appends `self` to the destination string.
    ///
    /// Time complexity: effectively O(n)
    pub fn push_to_string(&self, dst: &mut String) {
        for chunk in self.iter_chunks() {
            dst.push_str(chunk);
        }
    }

    /// Returns a slice of the string from the byte range [`start`..`end`).
    ///
    /// Time complexity: O(log n)
    // Maybe take Range arguments as well (would need traits)
    pub fn slice(self, mut start: usize, mut end: usize) -> Rope {
        let mut root = self.root;
        start += self.start;
        end += self.start;
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
            start: start,
            len: end - start
        }
    }

    /// Append `s` to the string.
    pub fn push_str(&mut self, s: &str) {
        let len = self.len();
        self.edit_str(len, len, s);
    }

    /// Edit the string, replacing the byte range [`start`..`end`] with `new`.
    ///
    /// Note: `edit` and `edit_str` may be merged, using traits.
    ///
    /// Time complexity: O(log n)
    pub fn edit(&mut self, start: usize, end: usize, new: Rope) {
        if self.is_full() {
            self.root.replace(start, end, new.normalize().root);
            self.len = self.root.len();
        } else {
            let mut b = RopeBuilder::new();
            self.root.subsequence_rec(&mut b, self.start, self.start + start);
            b.push_rope(new);
            self.root.subsequence_rec(&mut b, self.start + end, self.start + self.len);
            *self = b.build_rope()
        }
    }

    /// Edit the string, replacing the byte range [`start`..`end`] with `new`.
    ///
    /// Note: `edit` and `edit_str` may be merged, using traits.
    ///
    /// Time complexity: O(log n)
    pub fn edit_str(&mut self, start: usize, end: usize, new: &str) {
        if self.is_full() {
            self.root.replace_str(start, end, new);
            self.len = self.root.len();
        } else {
            let mut b = RopeBuilder::new();
            self.root.subsequence_rec(&mut b, self.start, self.start + start);
            b.push_str(new);
            self.root.subsequence_rec(&mut b, self.start + end, self.start + self.len);
            *self = b.build_rope()
        }
    }

    /// Returns an iterator over chunks of the rope.
    ///
    /// Each chunk is a `&str` slice borrowed from the rope's storage. The size
    /// of the chunks is indeterminate but for large strings will generally be
    /// in the range of 511-1024 bytes.
    ///
    /// The empty string will yield a single empty slice. In all other cases, the
    /// slices will be nonempty.
    ///
    /// Time complexity: technically O(n log n), but the constant factor is so
    /// tiny it is effectively O(n). This iterator does not allocate.
    pub fn iter_chunks(&self) -> ChunkIter {
        ChunkIter {
            root: &self.root,
            start: self.start,
            end: self.start + self.len,
            cache: [None; CHUNK_CACHE_SIZE],
            first: true
        }
    }

    // access to line structure

    /// Return the line number corresponding to the byte index `offset`.
    ///
    /// The line number is 0-based, thus this is equivalent to the count of newlines
    /// in the slice up to `offset`.
    ///
    /// Time complexity: O(log n)
    pub fn line_of_offset(&self, offset: usize) -> usize {
        if offset > self.len {
            panic!("offset out of range");
        }
        self.root.line_of_offset(offset + self.start) - self.root.line_of_offset(self.start)
    }

    /// Return the byte offset corresponding to the line number `line`.
    ///
    /// The line number is 0-based.
    ///
    /// Time complexity: O(log n)
    pub fn offset_of_line(&self, line: usize) -> usize {
        let start_line = self.root.line_of_offset(self.start);
        let result = self.root.offset_of_line(line + start_line) - self.start;
        // TODO: better checking of line out of range
        min(result, self.len)
    }

    /// An iterator over the raw lines. The lines, except the last, include the
    /// terminating newline.
    ///
    /// The return type is a `Cow<str>`, and in most cases the lines are slices borrowed
    /// from the rope.
    pub fn lines_raw(&self) -> LinesRaw {
        LinesRaw {
            inner: self.iter_chunks(),
            fragment: ""
        }
    }

    /// An iterator over the lines of a rope.
    ///
    /// Lines are ended with either Unix (`\n`) or MS-DOS (`\r\n`) style line endings.
    /// The line ending is stripped from the resulting string. The final line ending
    /// is optional.
    ///
    /// The return type is a `Cow<str>`, and in most cases the lines are slices borrowed
    /// from the rope.
    ///
    /// The semantics are intended to match `str::lines()`.
    pub fn lines(&self) -> Lines {
        Lines {
            inner: self.lines_raw()
        }
    }

    /// Return the offset of the codepoint before `offset`.
    pub fn prev_codepoint_offset(&self, offset: usize) -> Option<usize> {
        if offset == 0 || offset > self.len() {
            return None
        }
        Some(self.root.prev_codepoint_offset(offset + self.start) - self.start)
    }

    /// Return the offset of the codepoint after `offset`.
    pub fn next_codepoint_offset(&self, offset: usize) -> Option<usize> {
        if offset >= self.len() {
            return None
        }
        Some(self.root.next_codepoint_offset(offset + self.start) - self.start)
    }

    pub fn prev_grapheme_offset(&self, offset: usize) -> Option<usize> {
        // TODO: actual grapheme analysis
        self.prev_codepoint_offset(offset)
    }

    pub fn next_grapheme_offset(&self, offset: usize) -> Option<usize> {
        // TODO: actual grapheme analysis
        self.next_codepoint_offset(offset)
    }

    pub fn byte_at(&self, offset: usize) -> u8 {
        self.root.byte_at(offset + self.start)
    }

    // return condition: result is_full
    // TODO: maybe return a node, we always seem to use that?
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

impl<T: AsRef<str>> From<T> for Rope {
    fn from(s: T) -> Rope {
        Rope::from_node(Node::from_str(s.as_ref()).unwrap())
    }
}

impl From<Rope> for String {
    fn from(r: Rope) -> String {
        if r.is_full() {
            r.root.into_string()
        } else {
            String::from(&r)
        }
    }
}

impl<'a> From<&'a Rope> for String {
    fn from(r: &Rope) -> String {
        let mut result = String::new();
        r.push_to_string(&mut result);
        result
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
        let mut lhs = self.normalize();
        lhs.push_str(rhs.as_ref());
        lhs
    }
}

impl FromStr for Node {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Node, Self::Err> {
        let mut b = RopeBuilder::new();
        b.push_str(s);
        Ok(b.build())
    }
}

impl Node {
    fn new(n: NodeBody) -> Node {
        Node(Rc::new(n))
    }

    fn height(&self) -> usize {
        self.0.height
    }

    fn is_leaf(&self) -> bool {
        self.0.height == 0
    }

    pub fn len(&self) -> usize {
        self.0.len
    }

    fn newline_count(&self) -> usize {
        self.0.newline_count
    }

    fn get_children(&self) -> &[Node] {
        if let NodeVal::Internal(ref v) = self.0.val {
            v
        } else {
            panic!("get_children called on leaf node");
        }
    }

    fn get_leaf(&self) -> &str {
        if let NodeVal::Leaf(ref s) = self.0.val {
            s
        } else {
            panic!("get_leaf called on internal node");
        }
    }

    fn is_ok_child(&self) -> bool {
        match self.0.val {
            NodeVal::Leaf(_) => (self.len() >= MIN_LEAF),
            NodeVal::Internal(ref pieces) => (pieces.len() >= MIN_CHILDREN)
        }
    }

    fn from_string_piece(s: String) -> Node {
        debug_assert!(s.len() <= MAX_LEAF);

        Node::new(NodeBody {
            height: 0,
            len: s.len(),
            newline_count: count_newlines(&s),
            val: NodeVal::Leaf(s)
        })
    }

    fn from_pieces(pieces: Vec<Node>) -> Node {
        debug_assert!(2 <= pieces.len() && pieces.len() <= MAX_CHILDREN);

        Node::new(NodeBody {
            height: pieces[0].height() + 1,
            len: pieces.iter().fold(0, |sum, r| sum + r.len()),
            newline_count: pieces.iter().fold(0, |sum, r| sum + r.newline_count()),
            val: NodeVal::Internal(pieces)
        })
    }

    fn merge_nodes(children1: &[Node], children2: &[Node]) -> Node {
        let n_children = children1.len() + children2.len();
        if n_children <= MAX_CHILDREN {
            Node::from_pieces([children1, children2].concat())
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

    fn merge_leaves(rope1: Node, rope2: Node) -> Node {
        debug_assert!(rope1.is_leaf() && rope2.is_leaf());

        if rope1.len() >= MIN_LEAF && rope2.len() >= MIN_LEAF {
            return Node::from_pieces(vec![rope1, rope2]);
        }
        // TODO: try to reuse rope1 if uniquely owned
        match (&rope1.0.val, &rope2.0.val) {
            (&NodeVal::Leaf(ref s1), &NodeVal::Leaf(ref s2)) => {
                let mut s = [s1.as_str(), s2.as_str()].concat();
                if s.len() <= MAX_LEAF {
                    Node::from_string_piece(s)
                } else {
                    let splitpoint = find_leaf_split_for_merge(&s);
                    let right_str = s[splitpoint..].to_owned();
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
                    return Node::from_pieces(vec![rope1, rope2]);
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

    fn subsequence_rec(&self, b: &mut RopeBuilder, start: usize, end: usize) {
        if start == 0 && self.len() == end {
            b.push(self.clone());
            return
        }
        match self.0.val {
            NodeVal::Leaf(ref s) => b.push_str_short(&s[start..end]),
            NodeVal::Internal(ref v) => {
                let mut offset = 0;
                for child in v {
                    if end <= offset {
                        break;
                    }
                    if offset + child.len() > start {
                        //println!("start={}, end={}, offset={}, child.size={}", start, end, offset, child.size());
                        child.subsequence_rec(b, max(offset, start) - offset, min(child.len(), end - offset));
                    }
                    offset += child.len()
                }
            }
        }
    }

    fn replace(&mut self, start: usize, end: usize, new: Node) {
        if let NodeVal::Leaf(ref s) = new.0.val {
            if s.len() < MIN_LEAF {
                self.replace_str(start, end, s);
                return;
            }
        }
        let mut b = RopeBuilder::new();
        self.subsequence_rec(&mut b, 0, start);
        b.push(new);
        self.subsequence_rec(&mut b, end, self.len());
        *self = b.build()
    }

    fn replace_str(&mut self, start: usize, end: usize, new: &str) {
        // try to do replacement without changing tree structure
        if new.len() < MIN_LEAF && Node::try_replace_str(self, start, end, new) {
            return;
        }
        let mut b = RopeBuilder::new();
        self.subsequence_rec(&mut b, 0, start);
        b.push_str(new);
        self.subsequence_rec(&mut b, end, self.len());
        *self = b.build()
    }

    fn try_replace_leaf_str(n: &mut Node, start: usize, end: usize, new: &str) -> bool {
        debug_assert!(n.is_leaf());
        // TODO: maybe try to mutate in place, using either unsafe or
        // special-case single-char insert and remove (plus trunc, append)

        let size_plus_new = n.len() + new.len();
        if size_plus_new < MIN_LEAF + (end - start) || size_plus_new > MAX_LEAF + (end - start) {
            return false;
        }
        *n = {
            let s = n.get_leaf();
            let newstr = [&s[..start], new, &s[end..]].concat();
            Node::from_string_piece(newstr)
        };
        true
    }

    // return child index and offset on success
    fn try_find_child(children: &[Node], start: usize, end: usize) -> Option<(usize, usize)> {
        // TODO: maybe try scanning from back if close to end (would need parent's size)
        let mut offset = 0;
        let mut i = 0;
        while i < children.len() {
            let nextoff = offset + children[i].len();
            if nextoff >= start {
                if nextoff >= end {
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
                    if let Some((i, offset)) = Node::try_find_child(v, start, end) {
                        let old_nl_count = v[i].newline_count();
                        success = Node::try_replace_str(&mut v[i], start - offset, end - offset, new);
                        if success {
                            // update invariants
                            node.len = node.len - (end - start) + new.len();
                            node.newline_count = node.newline_count - old_nl_count + v[i].newline_count();
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
                    let v = [&children[..i], &[child][..], &children[i + 1 ..]].concat();
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
        match self.0.val {
            NodeVal::Leaf(ref s) => dst.push_str(s),
            NodeVal::Internal(ref v) => {
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

    // line access

    fn line_of_offset(&self, mut offset: usize) -> usize {
        if offset == 0 { return 0; }
        if offset == self.len() { return self.newline_count(); }
        let mut result = 0;
        let mut node = self;
        while node.height() > 0 {
            for child in node.get_children() {
                if child.len() > offset {
                    node = child;
                    break;
                }
                result += child.newline_count();
                offset -= child.len();
            }
        }
        result + count_newlines(&node.get_leaf()[..offset])
    }

    fn offset_of_line(&self, mut line: usize) -> usize {
        if line == 0 { return 0; }
        if line > self.newline_count() { return self.len(); }
        let mut result = 0;
        let mut node = self;
        while node.height() > 0 {
            for child in node.get_children() {
                if child.newline_count() >= line {
                    node = child;
                    break;
                }
                result += child.len();
                line -= child.newline_count();
            }
        }
        let mut s = node.get_leaf();
        while line > 0 {
            let i = s.as_bytes().iter().position(|&c| c == b'\n').unwrap() + 1;
            result += i;
            line -= 1;
            s = &s[i..];
        }
        result
    }

    // navigation

    // return is leaf and offset within leaf
    fn leaf_at(&self, mut offset: usize) -> (&str, usize) {
        let mut node = self;
        while node.height() > 0 {
            for child in node.get_children() {
                if child.len() > offset {
                    node = child;
                    break;
                }
                offset -= child.len();
            }
        }
        (node.get_leaf(), offset)
    }

    fn prev_codepoint_offset(&self, offset: usize) -> usize {
        debug_assert!(offset > 0 && offset <= self.len());

        let (s, try_offset) = self.leaf_at(offset - 1);
        let mut len = 1;
        while !is_char_boundary(s, try_offset + 1 - len) {
            len += 1;
        }
        offset - len
    }

    fn next_codepoint_offset(&self, offset: usize) -> usize {
        debug_assert!(offset < self.len());

        let (s, try_offset) = self.leaf_at(offset);
        let b = s.as_bytes()[try_offset];
        offset + match b {
            b if b < 0x80 => 1,
            b if b < 0xe0 => 2,
            b if b < 0xf0 => 3,
            _ => 4
        }
    }

    fn byte_at(&self, offset: usize) -> u8 {
        let (s, offset) = self.leaf_at(offset);
        s.as_bytes()[offset]
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

impl Default for RopeBuilder {
    fn default() -> RopeBuilder {
        RopeBuilder(None)
    }
}

impl RopeBuilder {
    fn new() -> RopeBuilder {
        RopeBuilder::default()
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

    fn push_str_short(&mut self, s: &str) {
        debug_assert!(s.len() <= MAX_LEAF);

        self.push(Node::from_string_piece(s.to_owned()));
    }

    fn push_str(&mut self, mut s: &str) {
        if s.len() <= MAX_LEAF {
            if !s.is_empty() {
                self.push_str_short(s);
            }
            return;
        }
        let mut stack: Vec<Vec<Node>> = Vec::new();
        while !s.is_empty() {
            let splitpoint = if s.len() > MAX_LEAF {
                find_leaf_split_for_bulk(s)
            } else {
                s.len()
            };
            let mut new = Node::from_string_piece(s[..splitpoint].to_owned());
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

// chunk iterator

const CHUNK_CACHE_SIZE: usize = 4;

pub struct ChunkIter<'a> {
    root: &'a Node,
    start: usize,  // advances
    end: usize,
    cache: [Option<(&'a Node, usize)>; CHUNK_CACHE_SIZE],
    first: bool
}

impl<'a> Iterator for ChunkIter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let start = self.start;
        if start >= self.end {
            return None;
        }
        if self.first {
            self.first = false;
            let (node, offset) = self.descend();
            return self.finish_leaf(node, offset);
        }
        let (node, j) = self.cache[0].unwrap();
        if j < node.get_children().len() {
            return self.finish_leaf(&node.get_children()[j], start);
        }
        for i in 1..CHUNK_CACHE_SIZE {
            let (node, j) = self.cache[i].unwrap();
            if j + 1 < node.get_children().len() {
                self.cache[i] = Some((node, j + 1));
                let mut node_down = &node.get_children()[j + 1];
                for k in (0..i).rev() {
                    self.cache[k] = Some((node_down, 0));
                    node_down = &node_down.get_children()[0];
                }
                return self.finish_leaf(node_down, start);
            }
        }
        let (node, offset) = self.descend();
        assert_eq!(offset, start);
        self.finish_leaf(node, offset)
    }
}

impl<'a> ChunkIter<'a> {
    // descend from root, filling cache. Return is leaf node and its offset.
    fn descend(&mut self) -> (&'a Node, usize) {
        let mut node = self.root;
        let mut offset = 0;
        while node.height() > 0 {
            let children = node.get_children();
            let mut i = 0;
            loop {
                let nextoff = offset + children[i].len();
                if nextoff > self.start {
                    break;
                }
                offset = nextoff;
                i += 1;
            }
            let cache_ix = node.height() - 1;
            if cache_ix < CHUNK_CACHE_SIZE {
                self.cache[cache_ix] = Some((node, i));
            }
            node = &children[i];
        }
        (node, offset)
    }

    fn finish_leaf(&mut self, node: &'a Node, offset: usize) -> Option<&'a str> {
        let s = node.get_leaf();
        let result = &s[self.start - offset .. min(s.len(), self.end - offset)];
        self.start += result.len();
        if self.start < self.end {
            let (node, j) = self.cache[0].unwrap();
            self.cache[0] = Some((node, j + 1));
        }
        Some(result)
    }
}

// line iterators

pub struct LinesRaw<'a> {
    inner: ChunkIter<'a>,
    fragment: &'a str
}

fn cow_append<'a>(a: Cow<'a, str>, b: &'a str) -> Cow<'a, str> {
    if a.is_empty() {
        Cow::from(b)
    } else {
        Cow::from(a.into_owned() + b)
    }
}

impl<'a> Iterator for LinesRaw<'a> {
    type Item = Cow<'a, str>;

    fn next(&mut self) -> Option<Cow<'a, str>> {
        let mut result = Cow::from("");
        loop {
            if self.fragment.is_empty() {
                match self.inner.next() {
                    Some(chunk) => self.fragment = chunk,
                    None => return if result.is_empty() { None } else { Some(result) }
                }
                if self.fragment.is_empty() {
                    // can only happen on empty input
                    return None;
                }
            }
            match self.fragment.as_bytes().iter().position(|&c| c == b'\n') {
                Some(i) => {
                    result = cow_append(result, &self.fragment[.. i + 1]);
                    self.fragment = &self.fragment[i + 1 ..];
                    return Some(result);
                },
                None => {
                    result = cow_append(result, self.fragment);
                    self.fragment = "";
                }
            }
        }
    }
}

pub struct Lines<'a> {
    inner: LinesRaw<'a>
}

impl<'a> Iterator for Lines<'a> {
    type Item = Cow<'a, str>;

    fn next(&mut self) -> Option<Cow<'a, str>> {
        match self.inner.next() {
            Some(Cow::Borrowed(mut s)) => {
                if s.ends_with('\n') {
                    s = &s[..s.len() - 1];
                    if s.ends_with('\r') {
                        s = &s[..s.len() - 1];
                    }
                }
                Some(Cow::from(s))
            },
            Some(Cow::Owned(mut s)) => {
                if s.ends_with('\n') {
                    let _ = s.pop();
                    if s.ends_with('\r') {
                        let _ = s.pop();
                    }
                }
                Some(Cow::from(s))
            }
            None => None
        }
    }
}

// Equality and related

fn eq_chunks<'a, T: Iterator<Item=&'a str>, U: Iterator<Item=&'a str>>(mut a: T, mut b: U) -> bool {
    let mut a_chunk = &b""[..];
    let mut b_chunk = &b""[..];
    loop {
        if a_chunk.is_empty() {
            if let Some(s) = a.next() { a_chunk = s.as_bytes(); }
        }
        if b_chunk.is_empty() {
            if let Some(s) = b.next() { b_chunk = s.as_bytes(); }
        }
        let len = min(a_chunk.len(), b_chunk.len());
        if len == 0 {
            return a_chunk.is_empty() && b_chunk.is_empty();
        }
        if a_chunk[..len] != b_chunk[..len] {
            return false;
        }
        a_chunk = &a_chunk[len..];
        b_chunk = &b_chunk[len..];
    }
}

impl PartialEq for Rope {
    fn eq(&self, rhs: &Rope) -> bool {
        self.len() == rhs.len() && eq_chunks(self.iter_chunks(), rhs.iter_chunks())
    }
}

impl Eq for Rope {
}

impl PartialEq<str> for Rope {
    fn eq(&self, rhs: &str) -> bool {
        self.len() == rhs.len() && eq_chunks(self.iter_chunks(), once(rhs))
    }
}

impl<'a> PartialEq<&'a str> for Rope {
    fn eq(&self, rhs: &&str) -> bool {
        self.len() == rhs.len() && eq_chunks(self.iter_chunks(), once(*rhs))
    }
}

impl PartialEq<String> for Rope {
    fn eq(&self, rhs: &String) -> bool {
        self.len() == rhs.len() && eq_chunks(self.iter_chunks(), once(rhs.as_str()))
    }
}

impl<'a> PartialEq<Cow<'a, str>> for Rope {
    fn eq(&self, rhs: &Cow<'a, str>) -> bool {
        self.len() == rhs.len() && eq_chunks(self.iter_chunks(), once(&**rhs))
    }
}

impl PartialEq<Rope> for str {
    fn eq(&self, rhs: &Rope) -> bool {
        rhs == self
    }
}

impl<'a> PartialEq<Rope> for &'a str {
    fn eq(&self, rhs: &Rope) -> bool {
        rhs == self
    }
}

impl PartialEq<Rope> for String {
    fn eq(&self, rhs: &Rope) -> bool {
        rhs == self
    }
}

impl<'a> PartialEq<Rope> for Cow<'a, str> {
    fn eq(&self, rhs: &Rope) -> bool {
        rhs == self
    }
}

#[test]
fn line_of_offset_small() {
    let a = Rope::from("a\nb\nc");
    assert_eq!(0, a.line_of_offset(0));
    assert_eq!(0, a.line_of_offset(1));
    assert_eq!(1, a.line_of_offset(2));
    assert_eq!(1, a.line_of_offset(3));
    assert_eq!(2, a.line_of_offset(4));
    assert_eq!(2, a.line_of_offset(5));
    let b = a.slice(2, 4);
    assert_eq!(0, b.line_of_offset(0));
    assert_eq!(0, b.line_of_offset(1));
    assert_eq!(1, b.line_of_offset(2));
}

#[test]
fn offset_of_line_small() {
    let a = Rope::from("a\nb\nc");
    assert_eq!(0, a.offset_of_line(0));
    assert_eq!(2, a.offset_of_line(1));
    assert_eq!(4, a.offset_of_line(2));
    assert_eq!(5, a.offset_of_line(3));
    let b = a.slice(2, 4);
    assert_eq!(0, b.offset_of_line(0));
    assert_eq!(2, b.offset_of_line(1));
}

#[test]
fn lines_raw_small() {
    let a = Rope::from("a\nb\nc");
    assert_eq!(vec!["a\n", "b\n", "c"], a.lines_raw().collect::<Vec<_>>());

    let a = Rope::from("a\nb\n");
    assert_eq!(vec!["a\n", "b\n"], a.lines_raw().collect::<Vec<_>>());

    let a = Rope::from("\n");
    assert_eq!(vec!["\n"], a.lines_raw().collect::<Vec<_>>());

    let a = Rope::from("");
    assert_eq!(0, a.lines_raw().count());
}

#[test]
fn lines_med() {
    let mut a = String::new();
    let mut b = String::new();
    let line_len = MAX_LEAF + MIN_LEAF - 1;
    for _ in 0..line_len {
        a.push('a');
        b.push('b');
    }
    a.push('\n');
    b.push('\n');
    let r = Rope::from(&a[..MAX_LEAF]);
    let r = r + Rope::from(String::from(&a[MAX_LEAF..]) + &b[..MIN_LEAF]);
    let r = r + Rope::from(&b[MIN_LEAF..]);
    //println!("{:?}", r.iter_chunks().collect::<Vec<_>>());

    assert_eq!(vec![a.as_str(), b.as_str()], r.lines_raw().collect::<Vec<_>>());
    assert_eq!(vec![&a[..line_len], &b[..line_len]], r.lines().collect::<Vec<_>>());
    assert_eq!(String::from(&r).lines().collect::<Vec<_>>(), r.lines().collect::<Vec<_>>());

    // additional tests for line indexing
    assert_eq!(a.len(), r.offset_of_line(1));
    assert_eq!(r.len(), r.offset_of_line(2));
    assert_eq!(0, r.line_of_offset(a.len() - 1));
    assert_eq!(1, r.line_of_offset(a.len()));
    assert_eq!(1, r.line_of_offset(r.len() - 1));
    assert_eq!(2, r.line_of_offset(r.len()));
}

#[test]
fn lines_small() {
    let a = Rope::from("a\nb\nc");
    assert_eq!(vec!["a", "b", "c"], a.lines().collect::<Vec<_>>());
    assert_eq!(String::from(&a).lines().collect::<Vec<_>>(), a.lines().collect::<Vec<_>>());

    let a = Rope::from("a\nb\n");
    assert_eq!(vec!["a", "b"], a.lines().collect::<Vec<_>>());
    assert_eq!(String::from(&a).lines().collect::<Vec<_>>(), a.lines().collect::<Vec<_>>());

    let a = Rope::from("\n");
    assert_eq!(vec![""], a.lines().collect::<Vec<_>>());
    assert_eq!(String::from(&a).lines().collect::<Vec<_>>(), a.lines().collect::<Vec<_>>());

    let a = Rope::from("");
    assert_eq!(0, a.lines().count());
    assert_eq!(String::from(&a).lines().collect::<Vec<_>>(), a.lines().collect::<Vec<_>>());

    let a = Rope::from("a\r\nb\r\nc");
    assert_eq!(vec!["a", "b", "c"], a.lines().collect::<Vec<_>>());
    assert_eq!(String::from(&a).lines().collect::<Vec<_>>(), a.lines().collect::<Vec<_>>());

    let a = Rope::from("a\rb\rc");
    assert_eq!(vec!["a\rb\rc"], a.lines().collect::<Vec<_>>());
    assert_eq!(String::from(&a).lines().collect::<Vec<_>>(), a.lines().collect::<Vec<_>>());
}

#[test]
fn append_large() {
    let mut a = Rope::from("");
    let mut b = String::new();
    for i in 0..5_000 {
        let c = i.to_string() + "\n";
        b.push_str(&c);
        a = a + c;
    }
    assert_eq!(b, String::from(a));
}

#[test]
fn eq_small() {
    let a = Rope::from("a");
    let a2 = Rope::from("a");
    let b = Rope::from("b");
    let empty = Rope::from("");
    assert!(a == a2);
    assert!(a != b);
    assert!(a != empty);
    assert!(empty == empty);
    assert!(a.slice(0, 0) == empty);
}

#[test]
fn eq_med() {
    let mut a = String::new();
    let mut b = String::new();
    let line_len = MAX_LEAF + MIN_LEAF - 1;
    for _ in 0..line_len {
        a.push('a');
        b.push('b');
    }
    a.push('\n');
    b.push('\n');
    let r = Rope::from(&a[..MAX_LEAF]);
    let r = r + Rope::from(String::from(&a[MAX_LEAF..]) + &b[..MIN_LEAF]);
    let r = r + Rope::from(&b[MIN_LEAF..]);

    let a_rope = Rope::from(&a);
    let b_rope = Rope::from(&b);
    assert!(r != a_rope);
    assert!(r.clone().slice(0, a.len()) == a_rope);
    assert!(r.clone().slice(a.len(), r.len()) == b_rope);
    assert!(r == a_rope.clone() + b_rope.clone());
    assert!(r != b_rope + a_rope);
}

#[test]
fn prev_codepoint_offset_small() {
    let a = Rope::from("a\u{00A1}\u{4E00}\u{1F4A9}");
    assert_eq!(Some(6), a.prev_codepoint_offset(10));
    assert_eq!(Some(3), a.prev_codepoint_offset(6));
    assert_eq!(Some(1), a.prev_codepoint_offset(3));
    assert_eq!(Some(0), a.prev_codepoint_offset(1));
    assert_eq!(None, a.prev_codepoint_offset(0));
    let b = a.slice(1, 10);
    assert_eq!(Some(5), b.prev_codepoint_offset(9));
    assert_eq!(Some(2), b.prev_codepoint_offset(5));
    assert_eq!(Some(0), b.prev_codepoint_offset(2));
    assert_eq!(None, b.prev_codepoint_offset(0));
}

#[test]
fn next_codepoint_offset_small() {
    let a = Rope::from("a\u{00A1}\u{4E00}\u{1F4A9}");
    assert_eq!(Some(10), a.next_codepoint_offset(6));
    assert_eq!(Some(6), a.next_codepoint_offset(3));
    assert_eq!(Some(3), a.next_codepoint_offset(1));
    assert_eq!(Some(1), a.next_codepoint_offset(0));
    assert_eq!(None, a.next_codepoint_offset(10));
    let b = a.slice(1, 10);
    assert_eq!(Some(9), b.next_codepoint_offset(5));
    assert_eq!(Some(5), b.next_codepoint_offset(2));
    assert_eq!(Some(2), b.next_codepoint_offset(0));
    assert_eq!(None, b.next_codepoint_offset(9));
}
