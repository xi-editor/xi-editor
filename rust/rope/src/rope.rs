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
use std::borrow::Cow;
use std::str::FromStr;
use std::string::ParseError;

use tree::{Leaf, Node, NodeInfo, Metric, TreeBuilder, Cursor};
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
}

impl NodeInfo for RopeInfo {
    type L = String;

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

#[derive(Clone, Copy)]
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
            while !is_char_boundary(s, offset - len) {
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

#[derive(Clone, Copy)]
pub struct LinesMetric(usize);  // number of lines

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
        memchr(b'\n', &s.as_bytes()[offset..])
            .map(|pos| offset + pos + 1)
    }

    fn can_fragment() -> bool { true }
}

// Low level functions

// TODO: use burntsushi memchr
pub fn memchr(needle: u8, haystack: &[u8]) -> Option<usize> {
    haystack.iter().position(|&b| b == needle)
}

// TODO: explore ways to make this faster - SIMD would be a big win
// memchr is probably best for now
fn count_newlines(s: &str) -> usize {
    s.as_bytes().iter().filter(|&&c| c == b'\n').count()
}

// TODO: probably will be stabilized in Rust std lib
// Note, this isn't exactly the same, it panics when index > s.len()
fn is_char_boundary(s: &str, index: usize) -> bool {
    // fancy bit magic for ranges 0..0x80 | 0xc0..
    index == s.len() || (s.as_bytes()[index] as i8) >= -0x40
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

// Additional APIs custom to strings

impl FromStr for Rope {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Rope, Self::Err> {
        let mut b = TreeBuilder::new();
        b.push_str(s);
        Ok(b.build())
    }
}

impl Rope {
    pub fn edit_str(&mut self, start: usize, end: usize, new: &str) {
        let mut b = TreeBuilder::new();
        // TODO: may make this method take the iv directly
        let edit_iv = Interval::new_closed_open(start, end);
        let self_iv = Interval::new_closed_closed(0, self.len());
        self.push_subseq(&mut b, self_iv.prefix(edit_iv));
        b.push_str(new);
        self.push_subseq(&mut b, self_iv.suffix(edit_iv));
        *self = b.build();
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

    // graphemes should probably be developed as a cursor-based interface
    pub fn prev_grapheme_offset(&self, offset: usize) -> Option<usize> {
        // TODO: actual grapheme analysis
        self.prev_codepoint_offset(offset)
    }

    pub fn next_grapheme_offset(&self, offset: usize) -> Option<usize> {
        // TODO: actual grapheme analysis
        self.next_codepoint_offset(offset)
    }

    /// Return the line number corresponding to the byte index `offset`.
    ///
    /// The line number is 0-based, thus this is equivalent to the count of newlines
    /// in the slice up to `offset`.
    ///
    /// Time complexity: O(log n)
    pub fn line_of_offset(&self, offset: usize) -> usize {
        self.convert_metrics::<BaseMetric, LinesMetric>(offset)
    }

    /// Return the byte offset corresponding to the line number `line`.
    ///
    /// The line number is 0-based.
    ///
    /// Time complexity: O(log n)
    pub fn offset_of_line(&self, line: usize) -> usize {
        if line > self.measure::<LinesMetric>() {
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
    pub fn iter_chunks(&self, start: usize, end: usize) -> ChunkIter {
        ChunkIter {
            cursor: Cursor::new(self, start),
            end: end,
        }
    }
    /// An iterator over the raw lines. The lines, except the last, include the
    /// terminating newline.
    ///
    /// The return type is a `Cow<str>`, and in most cases the lines are slices borrowed
    /// from the rope.
    pub fn lines_raw(&self, start: usize, end: usize) -> LinesRaw {
        LinesRaw {
            inner: self.iter_chunks(start, end),
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
    pub fn lines(&self, start: usize, end: usize) -> Lines {
        Lines {
            inner: self.lines_raw(start, end)
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
    pub fn slice_to_string(&self, start: usize, end: usize) -> String {
        let mut result = String::new();
        for chunk in self.iter_chunks(start, end) {
            result.push_str(chunk);
        }
        result
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
        Some(&leaf[start_pos .. start_pos + len])
    }
}

impl TreeBuilder<RopeInfo> {
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
        r.slice_to_string(0, r.len())
    }
}

// additional cursor features

impl<'a> Cursor<'a, RopeInfo> {
    pub fn next_codepoint(&mut self) -> Option<char> {
        if let Some((l, offset)) = self.get_leaf() {
            self.next::<BaseMetric>();
            l[offset..].chars().next()
        } else {
            None
        }
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
            match memchr(b'\n', self.fragment.as_bytes()) {
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

#[cfg(test)]
mod tests {
    use rope::Rope;

    #[test]
    fn replace_small() {
        let mut a = Rope::from("hello world");
        a.edit_str(1, 9, "era");
        assert_eq!("herald", String::from(a));
    }

    #[test]
    fn prev_codepoint_offset_small() {
        let a = Rope::from("a\u{00A1}\u{4E00}\u{1F4A9}");
        assert_eq!(Some(6), a.prev_codepoint_offset(10));
        assert_eq!(Some(3), a.prev_codepoint_offset(6));
        assert_eq!(Some(1), a.prev_codepoint_offset(3));
        assert_eq!(Some(0), a.prev_codepoint_offset(1));
        assert_eq!(None, a.prev_codepoint_offset(0));
        /* TODO
        let b = a.slice(1, 10);
        assert_eq!(Some(5), b.prev_codepoint_offset(9));
        assert_eq!(Some(2), b.prev_codepoint_offset(5));
        assert_eq!(Some(0), b.prev_codepoint_offset(2));
        assert_eq!(None, b.prev_codepoint_offset(0));
        */
    }

    #[test]
    fn next_codepoint_offset_small() {
        let a = Rope::from("a\u{00A1}\u{4E00}\u{1F4A9}");
        assert_eq!(Some(10), a.next_codepoint_offset(6));
        assert_eq!(Some(6), a.next_codepoint_offset(3));
        assert_eq!(Some(3), a.next_codepoint_offset(1));
        assert_eq!(Some(1), a.next_codepoint_offset(0));
        assert_eq!(None, a.next_codepoint_offset(10));
        /* TODO
        let b = a.slice(1, 10);
        assert_eq!(Some(9), b.next_codepoint_offset(5));
        assert_eq!(Some(5), b.next_codepoint_offset(2));
        assert_eq!(Some(2), b.next_codepoint_offset(0));
        assert_eq!(None, b.next_codepoint_offset(9));
        */
    }

}
