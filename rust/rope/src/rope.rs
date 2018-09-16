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

//! A rope data structure with a line count metric and (soon) other useful
//! info.

use std::borrow::Cow;
use std::cmp::{max, min};
use std::fmt;
use std::ops::Add;
use std::str;
use std::str::FromStr;
use std::string::ParseError;
use std::ops::Bound;
use std::ops::RangeBounds;

use delta::{Delta, DeltaElement};
use interval::Interval;
use tree::{Cursor, Leaf, Metric, Node, NodeInfo, TreeBuilder};

use bytecount;
use memchr::{memchr, memrchr};
use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, SerializeStruct, SerializeTupleVariant, Serializer};

use unicode_segmentation::GraphemeCursor;
use unicode_segmentation::GraphemeIncomplete;

const MIN_LEAF: usize = 511;
const MAX_LEAF: usize = 1024;

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
/// assert!("hello world" == String::from(a + b));
/// ```
///
/// Get a slice of a `Rope`:
///
/// ```rust
/// # use xi_rope::Rope;
/// let a = Rope::from("hello world");
/// let b = a.slice(1..9);
/// assert_eq!("ello wor", String::from(&b));
/// let c = b.slice(1..7);
/// assert_eq!("llo wo", String::from(c));
/// ```
///
/// Replace part of a `Rope`:
///
/// ```rust
/// # use xi_rope::Rope;
/// let mut a = Rope::from("hello world");
/// a.edit_str(1..9, "era");
/// assert_eq!("herald", String::from(a));
/// ```
pub type Rope = Node<RopeInfo>;

/// Represents a transform from one rope to another.
pub type RopeDelta = Delta<RopeInfo>;

/// An element in a `RopeDelta`.
pub type RopeDeltaElement = DeltaElement<RopeInfo>;

impl Leaf for String {
    fn len(&self) -> usize {
        self.len()
    }

    fn is_ok_child(&self) -> bool {
        self.len() >= MIN_LEAF
    }

    fn push_maybe_split(&mut self, other: &String, iv: Interval) -> Option<String> {
        //println!("push_maybe_split [{}] [{}] {:?}", self, other, iv);
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

#[derive(Clone, Copy)]
pub struct RopeInfo {
    lines: usize,
    utf16_size: usize,
}

impl NodeInfo for RopeInfo {
    type L = String;

    fn accumulate(&mut self, other: &Self) {
        self.lines += other.lines;
        self.utf16_size += other.utf16_size;
    }

    fn compute_info(s: &String) -> Self {
        RopeInfo {
            lines: count_newlines(s),
            utf16_size: count_utf16_code_units(s),
        }
    }

    fn identity() -> Self {
        RopeInfo {
            lines: 0,
            utf16_size: 0,
        }
    }
}

//TODO: document metrics, based on https://github.com/google/xi-editor/issues/456
//See ../docs/MetricsAndBoundaries.md for more information.
#[derive(Clone, Copy)]
pub struct BaseMetric(());

/// Measured unit is utf8 code unit.
/// Base unit is utf8 code unit.
/// Boundary is atomic and determined by codepoint boundary.
/// Atomicity is implicit, putting the offset
/// between two utf8 code units that form a code point is considered invalid.
/// For example, take a string that starts with a 0xC2 byte.
/// Then offset=1 is invalid.
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
        s.is_char_boundary(offset)
    }

    fn prev(s: &String, offset: usize) -> Option<usize> {
        if offset == 0 {
            // I think it's a precondition that this will never be called
            // with offset == 0, but be defensive.
            None
        } else {
            let mut len = 1;
            while !s.is_char_boundary(offset - len) {
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
            Some(offset + len_utf8_from_first_byte(b))
        }
    }

    fn can_fragment() -> bool {
        false
    }
}

/// Given the inital byte of a UTF-8 codepoint, returns the number of
/// bytes required to represent the codepoint.
/// RFC reference : https://tools.ietf.org/html/rfc3629#section-4
pub fn len_utf8_from_first_byte(b: u8) -> usize {
    match b {
        b if b < 0x80 => 1,
        b if b < 0xe0 => 2,
        b if b < 0xf0 => 3,
        _ => 4,
    }
}

#[derive(Clone, Copy)]
pub struct LinesMetric(usize); // number of lines

/// Measured unit is newline amount.
/// Base unit is utf8 code unit.
/// Boundary is trailing and determined by a newline char.
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
            match memchr(b'\n', &s.as_bytes()[offset..]) {
                Some(pos) => offset += pos + 1,
                _ => panic!("to_base_units called with arg too large"),
            }
        }
        offset
    }

    fn from_base_units(s: &String, in_base_units: usize) -> usize {
        count_newlines(&s[..in_base_units])
    }

    fn prev(s: &String, offset: usize) -> Option<usize> {
        memrchr(b'\n', &s.as_bytes()[..offset]).map(|pos| pos + 1)
    }

    fn next(s: &String, offset: usize) -> Option<usize> {
        memchr(b'\n', &s.as_bytes()[offset..]).map(|pos| offset + pos + 1)
    }

    fn can_fragment() -> bool {
        true
    }
}

#[derive(Clone, Copy)]
pub struct Utf16CodeUnitsMetric(usize);

impl Metric<RopeInfo> for Utf16CodeUnitsMetric {
    fn measure(info: &RopeInfo, _: usize) -> usize {
        info.utf16_size
    }

    fn is_boundary(s: &String, offset: usize) -> bool {
        s.is_char_boundary(offset)
    }

    fn to_base_units(s: &String, in_measured_units: usize) -> usize {
        let mut cur_len_utf16 = 0;
        let mut cur_len_utf8 = 0;
        for u in s.chars() {
            if cur_len_utf16 >= in_measured_units {
                break;
            }
            cur_len_utf16 += u.len_utf16();
            cur_len_utf8 += u.len_utf8();
        }
        cur_len_utf8
    }

    fn from_base_units(s: &String, in_base_units: usize) -> usize {
        count_utf16_code_units(&s[..in_base_units])
    }

    fn prev(s: &String, offset: usize) -> Option<usize> {
        if offset == 0 {
            // I think it's a precondition that this will never be called
            // with offset == 0, but be defensive.
            None
        } else {
            let mut len = 1;
            while !s.is_char_boundary(offset - len) {
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
            Some(offset + len_utf8_from_first_byte(b))
        }
    }

    fn can_fragment() -> bool {
        false
    }
}

// Low level functions

pub fn count_newlines(s: &str) -> usize {
    bytecount::count(s.as_bytes(), b'\n')
}

fn count_utf16_code_units(s: &str) -> usize {
    let mut utf16_count = 0;
    for &b in s.as_bytes() {
        if (b as i8) >= -0x40 {
            utf16_count += 1;
        }
        if b >= 0xf0 {
            utf16_count += 1;
        }
    }
    utf16_count
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
    match memrchr(b'\n', &s.as_bytes()[minsplit - 1..splitpoint]) {
        Some(pos) => minsplit + pos,
        None => {
            while !s.is_char_boundary(splitpoint) {
                splitpoint -= 1;
            }
            splitpoint
        }
    }
}

// Additional APIs custom to strings

impl FromStr for Rope {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Rope, Self::Err> {
        let mut b = TreeBuilder::new();
        b.push_str(s);
        Ok(b.build())
    }
}

impl Serialize for Rope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&String::from(self))
    }
}

impl<'de> Deserialize<'de> for Rope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(Rope::from(s))
    }
}

impl Serialize for DeltaElement<RopeInfo> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match *self {
            DeltaElement::Copy(ref start, ref end) => {
                let mut el = serializer.serialize_tuple_variant("DeltaElement", 0, "copy", 2)?;
                el.serialize_field(start)?;
                el.serialize_field(end)?;
                el.end()
            }
            DeltaElement::Insert(ref node) => {
                serializer.serialize_newtype_variant("DeltaElement", 1, "insert", node)
            }
        }
    }
}

impl Serialize for Delta<RopeInfo> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut delta = serializer.serialize_struct("Delta", 2)?;
        delta.serialize_field("els", &self.els)?;
        delta.serialize_field("base_len", &self.base_len)?;
        delta.end()
    }
}

impl<'de> Deserialize<'de> for Delta<RopeInfo> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // NOTE: we derive to an interim representation and then convert
        // that into our actual target.
        #[derive(Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum RopeDeltaElement_ {
            Copy(usize, usize),
            Insert(String),
        }

        #[derive(Serialize, Deserialize)]
        struct RopeDelta_ {
            els: Vec<RopeDeltaElement_>,
            base_len: usize,
        }

        impl From<RopeDeltaElement_> for DeltaElement<RopeInfo> {
            fn from(elem: RopeDeltaElement_) -> DeltaElement<RopeInfo> {
                match elem {
                    RopeDeltaElement_::Copy(start, end) => DeltaElement::Copy(start, end),
                    RopeDeltaElement_::Insert(s) => DeltaElement::Insert(Rope::from(s)),
                }
            }
        }

        impl From<RopeDelta_> for Delta<RopeInfo> {
            fn from(mut delta: RopeDelta_) -> Delta<RopeInfo> {
                Delta {
                    els: delta.els.drain(..).map(DeltaElement::from).collect(),
                    base_len: delta.base_len,
                }
            }
        }
        let d = RopeDelta_::deserialize(deserializer)?;
        Ok(Delta::from(d))
    }
}

impl Rope {
    /// Edit the string, replacing the byte range [`start`..`end`] with `new`.
    ///
    /// Note: `edit` and `edit_str` may be merged, using traits.
    ///
    /// Time complexity: O(log n)
    pub fn edit_str<T>(&mut self, range: T, new: &str) 
        where T: RangeBounds<usize> 
    {
        let (start, end) = self.extract_range(range);

        let mut b = TreeBuilder::new();
        // TODO: may make this method take the iv directly
        let edit_iv = Interval::new_closed_open(start, end);
        let self_iv = Interval::new_closed_closed(0, self.len());
        self.push_subseq(&mut b, self_iv.prefix(edit_iv));
        b.push_str(new);
        self.push_subseq(&mut b, self_iv.suffix(edit_iv));
        *self = b.build();
    }

    /// Returns a new Rope with the contents of the provided range.
    pub fn slice<T>(&self, range: T) -> Rope 
        where T: RangeBounds<usize>
    {
        let (start, end) = self.extract_range(range);

        let iv = Interval::new_closed_open(start, end);
        self.subseq(iv)
    }

    // encourage callers to use Cursor instead?

    /// Determine whether `offset` lies on a codepoint boundary.
    pub fn is_codepoint_boundary(&self, offset: usize) -> bool {
        let mut cursor = Cursor::new(self, offset);
        cursor.is_boundary::<BaseMetric>()
    }

    /// Return the offset of the codepoint before `offset`.
    pub fn prev_codepoint_offset(&self, offset: usize) -> Option<usize> {
        let mut cursor = Cursor::new(self, offset);
        cursor.prev::<BaseMetric>()
    }

    /// Return the offset of the codepoint after `offset`.
    pub fn next_codepoint_offset(&self, offset: usize) -> Option<usize> {
        let mut cursor = Cursor::new(self, offset);
        cursor.next::<BaseMetric>()
    }

    pub fn prev_grapheme_offset(&self, offset: usize) -> Option<usize> {
        let mut cursor = Cursor::new(self, offset);
        cursor.prev_grapheme()
    }

    pub fn next_grapheme_offset(&self, offset: usize) -> Option<usize> {
        let mut cursor = Cursor::new(self, offset);
        cursor.next_grapheme()
    }

    /// Return the line number corresponding to the byte index `offset`.
    ///
    /// The line number is 0-based, thus this is equivalent to the count of newlines
    /// in the slice up to `offset`.
    ///
    /// Time complexity: O(log n)
    ///
    /// # Panics
    ///
    /// This function will panic if `offset > self.len()`. Callers are expected to
    /// validate their input.
    pub fn line_of_offset(&self, offset: usize) -> usize {
        self.convert_metrics::<BaseMetric, LinesMetric>(offset)
    }

    /// Return the byte offset corresponding to the line number `line`.
    /// If `line` is equal to one plus the current number of lines,
    /// this returns the offset of the end of the rope. Arguments higher
    /// than this will panic.
    ///
    /// The line number is 0-based.
    ///
    /// Time complexity: O(log n)
    ///
    /// # Panics
    ///
    /// This function will panic if `line > self.measure::<LinesMetric>() + 1`.
    /// Callers are expected to validate their input.
    pub fn offset_of_line(&self, line: usize) -> usize {
        let max_line = self.measure::<LinesMetric>() + 1;
        if line > max_line {
            panic!("line number {} beyond last line {}", line, max_line);
        } else if line == max_line {
            return self.len();
        }
        self.convert_metrics::<LinesMetric, BaseMetric>(line)
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
    pub fn iter_chunks<T>(&self, range: T) -> ChunkIter 
        where T: RangeBounds<usize>
    {
        let (start, end) = self.extract_range(range);

        ChunkIter {
            cursor: Cursor::new(self, start),
            end: end,
        }
    }

    /// An iterator over the raw lines. The lines, except the last, include the
    /// terminating newline.
    ///
    /// The return type is a `Cow<str>`, and in most cases the lines are slices
    /// borrowed from the rope.
    pub fn lines_raw<T>(&self, range: T) -> LinesRaw 
        where T: RangeBounds<usize>
    {
        LinesRaw {
            inner: self.iter_chunks(range),
            fragment: "",
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
    pub fn lines<T>(&self, range: T) -> Lines
        where T: RangeBounds<usize>
    {
        Lines {
            inner: self.lines_raw(range)
        }
    }

    // callers should be encouraged to use cursor instead
    pub fn byte_at(&self, offset: usize) -> u8 {
        let cursor = Cursor::new(self, offset);
        let (leaf, pos) = cursor.get_leaf().unwrap();
        leaf.as_bytes()[pos]
    }

    // TODO: this should be a Cow
    // TODO: a case can be made to hang this on Cursor instead
    pub fn slice_to_string<T>(&self, range: T) -> String 
        where T: RangeBounds<usize>
    {
        let mut result = String::new();
        for chunk in self.iter_chunks(range) {
            result.push_str(chunk);
        }
        result
    }
    
    /// Extracts start and end bounds from a range
    fn extract_range<T>(&self, range: T) -> (usize, usize)
        where T: RangeBounds<usize>
    {
        let start = match range.start_bound() {
            Bound::Included(n) => *n,
            Bound::Excluded(n) => *n + 1,
            Bound::Unbounded => 0,
        };

        let end = match range.end_bound() {
            Bound::Included(n) => *n + 1,
            Bound::Excluded(n) => *n,
            Bound::Unbounded => self.len(),
        };

        (start, end)
    }
}

// should make this generic, but most leaf types aren't going to be sliceable
pub struct ChunkIter<'a> {
    cursor: Cursor<'a, RopeInfo>,
    end: usize,
}

impl<'a> Iterator for ChunkIter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        if self.cursor.pos() >= self.end {
            return None;
        }
        let (leaf, start_pos) = self.cursor.get_leaf().unwrap();
        let len = min(self.end - self.cursor.pos(), leaf.len() - start_pos);
        self.cursor.next_leaf();
        Some(&leaf[start_pos..start_pos + len])
    }
}

impl TreeBuilder<RopeInfo> {
    /// Push a string on the accumulating tree in the naive way.
    ///
    /// Splits the provided string in chunks that fit in a leaf
    /// and pushes the leaves one by one onto the tree by calling.
    pub fn push_str(&mut self, mut s: &str) {
        if s.len() <= MAX_LEAF {
            if !s.is_empty() {
                self.push_leaf(s.to_owned());
            }
            return;
        }
        while !s.is_empty() {
            let splitpoint = if s.len() > MAX_LEAF {
                find_leaf_split_for_bulk(s)
            } else {
                s.len()
            };
            self.push_leaf(s[..splitpoint].to_owned());
            s = &s[splitpoint..];
        }
    }

    /// Push a string on the accumulating tree in an optimized fashion.
    ///
    /// Splits the string into leaves first and
    /// then pushes all the leaves onto the accumulating tree in one go.
    ///
    /// Note: this is only used in tests.
    #[doc(hidden)]
    pub fn push_str_stacked(&mut self, s: &str) {
        let leaves = split_as_leaves(s);
        self.push_leaves(leaves);
    }
}

fn split_as_leaves(mut s: &str) -> Vec<String> {
    let mut nodes = Vec::new();
    while !s.is_empty() {
        let splitpoint = if s.len() > MAX_LEAF {
            find_leaf_split_for_bulk(s)
        } else {
            s.len()
        };
        nodes.push(s[..splitpoint].to_owned());
        s = &s[splitpoint..];
    }

    return nodes;
}

impl<T: AsRef<str>> From<T> for Rope {
    fn from(s: T) -> Rope {
        Rope::from_str(s.as_ref()).unwrap()
    }
}

impl From<Rope> for String {
    // maybe explore grabbing leaf? would require api in tree
    fn from(r: Rope) -> String {
        String::from(&r)
    }
}

impl<'a> From<&'a Rope> for String {
    fn from(r: &Rope) -> String {
        r.slice_to_string(..)
    }
}

impl fmt::Debug for Rope {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if f.alternate() {
            write!(f, "{}", String::from(self))
        } else {
            write!(f, "Rope({:?})", String::from(self))
        }
    }
}

impl Add<Rope> for Rope {
    type Output = Rope;
    fn add(self, rhs: Rope) -> Rope {
        let mut b = TreeBuilder::new();
        b.push(self);
        b.push(rhs);
        b.build()
    }
}

//additional cursor features

impl<'a> Cursor<'a, RopeInfo> {
    /// Get previous codepoint before cursor position, and advance cursor backwards.
    pub fn prev_codepoint(&mut self) -> Option<char> {
        self.prev::<BaseMetric>();
        if let Some((l, offset)) = self.get_leaf() {
            l[offset..].chars().next()
        } else {
            None
        }
    }

    /// Get next codepoint after cursor position, and advance cursor.
    pub fn next_codepoint(&mut self) -> Option<char> {
        if let Some((l, offset)) = self.get_leaf() {
            self.next::<BaseMetric>();
            l[offset..].chars().next()
        } else {
            None
        }
    }

    pub fn next_grapheme(&mut self) -> Option<usize> {
        let (mut l, mut offset) = self.get_leaf()?;
        let mut pos = self.pos();
        while offset < l.len() && !l.is_char_boundary(offset) {
            pos -= 1;
            offset -= 1;
        }
        let mut leaf_offset = pos - offset;
        let mut c = GraphemeCursor::new(pos, self.total_len(), true);
        let mut next_boundary = c.next_boundary(&l, leaf_offset);
        while let Err(incomp) = next_boundary {
            if let GraphemeIncomplete::PreContext(_) = incomp {
                let (pl, poffset) = self.prev_leaf()?;
                c.provide_context(&pl, self.pos() - poffset);
            } else if incomp == GraphemeIncomplete::NextChunk {
                self.set(pos);
                let (nl, noffset) = self.next_leaf()?;
                l = nl;
                leaf_offset = self.pos() - noffset;
                pos = leaf_offset + nl.len();
            } else {
                return None;
            }
            next_boundary = c.next_boundary(&l, leaf_offset);
        }
        next_boundary.unwrap_or(None)
    }

    pub fn prev_grapheme(&mut self) -> Option<usize> {
        let (mut l, mut offset) = self.get_leaf()?;
        let mut pos = self.pos();
        while offset < l.len() && !l.is_char_boundary(offset) {
            pos += 1;
            offset += 1;
        }
        let mut leaf_offset = pos - offset;
        let mut c = GraphemeCursor::new(pos, l.len() + leaf_offset, true);
        let mut prev_boundary = c.prev_boundary(&l, leaf_offset);
        while let Err(incomp) = prev_boundary {
            if let GraphemeIncomplete::PreContext(_) = incomp {
                let (pl, poffset) = self.prev_leaf()?;
                c.provide_context(&pl, self.pos() - poffset);
            } else if incomp == GraphemeIncomplete::PrevChunk {
                self.set(pos);
                let (pl, poffset) = self.prev_leaf()?;
                l = pl;
                leaf_offset = self.pos() - poffset;
                pos = leaf_offset + pl.len();
            } else {
                return None;
            }
            prev_boundary = c.prev_boundary(&l, leaf_offset);
        }
        prev_boundary.unwrap_or(None)
    }
}

// line iterators

pub struct LinesRaw<'a> {
    inner: ChunkIter<'a>,
    fragment: &'a str,
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
                    None => {
                        return if result.is_empty() {
                            None
                        } else {
                            Some(result)
                        }
                    }
                }
                if self.fragment.is_empty() {
                    // can only happen on empty input
                    return None;
                }
            }
            match memchr(b'\n', self.fragment.as_bytes()) {
                Some(i) => {
                    result = cow_append(result, &self.fragment[..i + 1]);
                    self.fragment = &self.fragment[i + 1..];
                    return Some(result);
                }
                None => {
                    result = cow_append(result, self.fragment);
                    self.fragment = "";
                }
            }
        }
    }
}

pub struct Lines<'a> {
    inner: LinesRaw<'a>,
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
            }
            Some(Cow::Owned(mut s)) => {
                if s.ends_with('\n') {
                    let _ = s.pop();
                    if s.ends_with('\r') {
                        let _ = s.pop();
                    }
                }
                Some(Cow::from(s))
            }
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_test::{assert_tokens, Token};

    #[test]
    fn replace_small() {
        let mut a = Rope::from("hello world");
        a.edit_str(1..9, "era");
        assert_eq!("herald", String::from(a));
    }

    #[test]
    fn lines_raw_small() {
        let a = Rope::from("a\nb\nc");
        assert_eq!(vec!["a\n", "b\n", "c"], a.lines_raw(..).collect::<Vec<_>>());
        assert_eq!(vec!["a\n", "b\n", "c"], a.lines_raw(..).collect::<Vec<_>>());

        let a = Rope::from("a\nb\n");
        assert_eq!(vec!["a\n", "b\n"], a.lines_raw(..).collect::<Vec<_>>());

        let a = Rope::from("\n");
        assert_eq!(vec!["\n"], a.lines_raw(..).collect::<Vec<_>>());

        let a = Rope::from("");
        assert_eq!(0, a.lines_raw(..).count());
    }

    #[test]
    fn lines_small() {
        let a = Rope::from("a\nb\nc");
        assert_eq!(vec!["a", "b", "c"], a.lines(..).collect::<Vec<_>>());
        assert_eq!(String::from(&a).lines().collect::<Vec<_>>(),
        a.lines(..).collect::<Vec<_>>());

        let a = Rope::from("a\nb\n");
        assert_eq!(vec!["a", "b"], a.lines(..).collect::<Vec<_>>());
        assert_eq!(String::from(&a).lines().collect::<Vec<_>>(),
        a.lines(..).collect::<Vec<_>>());

        let a = Rope::from("\n");
        assert_eq!(vec![""], a.lines(..).collect::<Vec<_>>());
        assert_eq!(String::from(&a).lines().collect::<Vec<_>>(),
        a.lines(..).collect::<Vec<_>>());

        let a = Rope::from("");
        assert_eq!(0, a.lines(..).count());
        assert_eq!(String::from(&a).lines().collect::<Vec<_>>(),
        a.lines(..).collect::<Vec<_>>());

        let a = Rope::from("a\r\nb\r\nc");
        assert_eq!(vec!["a", "b", "c"], a.lines(..).collect::<Vec<_>>());
        assert_eq!(String::from(&a).lines().collect::<Vec<_>>(),
        a.lines(..).collect::<Vec<_>>());

        let a = Rope::from("a\rb\rc");
        assert_eq!(vec!["a\rb\rc"], a.lines(..).collect::<Vec<_>>());
        assert_eq!(String::from(&a).lines().collect::<Vec<_>>(),
               a.lines(..).collect::<Vec<_>>());
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

        assert_eq!(vec![a.as_str(), b.as_str()], r.lines_raw(..).collect::<Vec<_>>());
        assert_eq!(vec![&a[..line_len], &b[..line_len]], r.lines(..).collect::<Vec<_>>());
        assert_eq!(String::from(&r).lines().collect::<Vec<_>>(),
                   r.lines(..).collect::<Vec<_>>());

        // additional tests for line indexing
        assert_eq!(a.len(), r.offset_of_line(1));
        assert_eq!(r.len(), r.offset_of_line(2));
        assert_eq!(0, r.line_of_offset(a.len() - 1));
        assert_eq!(1, r.line_of_offset(a.len()));
        assert_eq!(1, r.line_of_offset(r.len() - 1));
        assert_eq!(2, r.line_of_offset(r.len()));
    }

    #[test]
    fn append_large() {
        let mut a = Rope::from("");
        let mut b = String::new();
        for i in 0..5_000 {
            let c = i.to_string() + "\n";
            b.push_str(&c);
            a = a + Rope::from(&c);
        }
        assert_eq!(b, String::from(a));
    }

    #[test]
    fn prev_codepoint_offset_small() {
        let a = Rope::from("a\u{00A1}\u{4E00}\u{1F4A9}");
        assert_eq!(Some(6), a.prev_codepoint_offset(10));
        assert_eq!(Some(3), a.prev_codepoint_offset(6));
        assert_eq!(Some(1), a.prev_codepoint_offset(3));
        assert_eq!(Some(0), a.prev_codepoint_offset(1));
        assert_eq!(None, a.prev_codepoint_offset(0));
        let b = a.slice(1..10);
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
        let b = a.slice(1..10);
        assert_eq!(Some(9), b.next_codepoint_offset(5));
        assert_eq!(Some(5), b.next_codepoint_offset(2));
        assert_eq!(Some(2), b.next_codepoint_offset(0));
        assert_eq!(None, b.next_codepoint_offset(9));
    }

    #[test]
    fn prev_grapheme_offset() {
        // A with ring, hangul, regional indicator "US"
        let a = Rope::from("A\u{030a}\u{110b}\u{1161}\u{1f1fa}\u{1f1f8}");
        assert_eq!(Some(9), a.prev_grapheme_offset(17));
        assert_eq!(Some(3), a.prev_grapheme_offset(9));
        assert_eq!(Some(0), a.prev_grapheme_offset(3));
        assert_eq!(None, a.prev_grapheme_offset(0));
    }

    #[test]
    fn next_grapheme_offset() {
        // A with ring, hangul, regional indicator "US"
        let a = Rope::from("A\u{030a}\u{110b}\u{1161}\u{1f1fa}\u{1f1f8}");
        assert_eq!(Some(3), a.next_grapheme_offset(0));
        assert_eq!(Some(9), a.next_grapheme_offset(3));
        assert_eq!(Some(17), a.next_grapheme_offset(9));
        assert_eq!(None, a.next_grapheme_offset(17));
    }

    #[test]
    fn next_grapheme_offset_with_ris_of_leaf_boundaries() {
        let s1 = "\u{1f1fa}\u{1f1f8}".repeat(100);
        let a = Rope::concat(
            Rope::from(s1.clone()),
            Rope::concat(
                Rope::from(String::from(s1.clone()) + "\u{1f1fa}"),
                Rope::from(s1.clone()),
            ),
        );
        for i in 1..(s1.len() * 3) {
            assert_eq!(Some((i - 1) / 8 * 8), a.prev_grapheme_offset(i));
            assert_eq!(Some(i / 8 * 8 + 8), a.next_grapheme_offset(i));
        }
        for i in (s1.len() * 3 + 1)..(s1.len() * 3 + 4) {
            assert_eq!(Some(s1.len() * 3), a.prev_grapheme_offset(i));
            assert_eq!(Some(s1.len() * 3 + 4), a.next_grapheme_offset(i));
        }
        assert_eq!(None, a.prev_grapheme_offset(0));
        assert_eq!(Some(8), a.next_grapheme_offset(0));
        assert_eq!(Some(s1.len() * 3), a.prev_grapheme_offset(s1.len() * 3 + 4));
        assert_eq!(None, a.next_grapheme_offset(s1.len() * 3 + 4));
    }

    #[test]
    fn test_ser_de() {
        let rope = Rope::from("a\u{00A1}\u{4E00}\u{1F4A9}");
        assert_tokens(&rope, &[Token::Str("a\u{00A1}\u{4E00}\u{1F4A9}")]);
        assert_tokens(&rope, &[Token::String("a\u{00A1}\u{4E00}\u{1F4A9}")]);
        assert_tokens(&rope, &[Token::BorrowedStr("a\u{00A1}\u{4E00}\u{1F4A9}")]);
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
        let b = a.slice(2..4);
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
        let b = a.slice(2..4);
        assert_eq!(0, b.offset_of_line(0));
        assert_eq!(2, b.offset_of_line(1));
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
        assert!(a.slice(0..0) == empty);
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
        assert!(r.clone().slice(..a.len()) == a_rope);
        assert!(r.clone().slice(a.len()..) == b_rope);
        assert!(r == a_rope.clone() + b_rope.clone());
        assert!(r != b_rope + a_rope);
    }

    #[test]
    fn line_offsets() {
        let rope = Rope::from("hi\ni'm\nfour\nlines");
        assert_eq!(rope.offset_of_line(0), 0);
        assert_eq!(rope.offset_of_line(1), 3);
        assert_eq!(rope.line_of_offset(0), 0);
        assert_eq!(rope.line_of_offset(3), 1);
        // interior of first line should be first line
        assert_eq!(rope.line_of_offset(1), 0);
        // interior of last line should be last line
        assert_eq!(rope.line_of_offset(15), 3);
        assert_eq!(rope.offset_of_line(4), rope.len());
    }

    #[test]
    #[should_panic]
    fn line_of_offset_panic() {
        let rope = Rope::from("hi\ni'm\nfour\nlines");
        rope.line_of_offset(20);
    }

    #[test]
    #[should_panic]
    fn offset_of_line_panic() {
        let rope = Rope::from("hi\ni'm\nfour\nlines");
        rope.offset_of_line(5);
    }

    #[test]
    fn utf16_code_units_metric() {
        let rope = Rope::from("hi\ni'm\nfour\nlines");
        let utf16_units = rope.measure::<Utf16CodeUnitsMetric>();
        assert_eq!(utf16_units, 17);

        // position after 'f' in four
        let utf8_offset = 9;
        let utf16_units = rope.convert_metrics::<BaseMetric, Utf16CodeUnitsMetric>(utf8_offset);
        assert_eq!(utf16_units, 9);

        let utf8_offset = rope.convert_metrics::<Utf16CodeUnitsMetric, BaseMetric>(utf16_units);
        assert_eq!(utf8_offset, 9);

        let rope_with_emoji = Rope::from("hi\ni'm\n😀 four\nlines");
        let utf16_units = rope_with_emoji.measure::<Utf16CodeUnitsMetric>();

        assert_eq!(utf16_units, 20);

        // position after 'f' in four
        let utf8_offset = 13;
        let utf16_units =
            rope_with_emoji.convert_metrics::<BaseMetric, Utf16CodeUnitsMetric>(utf8_offset);
        assert_eq!(utf16_units, 11);

        let utf8_offset =
            rope_with_emoji.convert_metrics::<Utf16CodeUnitsMetric, BaseMetric>(utf16_units);
        assert_eq!(utf8_offset, 13);

        //for next line
        let utf8_offset = 19;
        let utf16_units =
            rope_with_emoji.convert_metrics::<BaseMetric, Utf16CodeUnitsMetric>(utf8_offset);
        assert_eq!(utf16_units, 17);

        let utf8_offset =
            rope_with_emoji.convert_metrics::<Utf16CodeUnitsMetric, BaseMetric>(utf16_units);
        assert_eq!(utf8_offset, 19);
    }
}
