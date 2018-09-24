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

//! Unicode utilities useful for text editing, including a line breaking iterator.

mod tables;

use tables::*;

pub fn linebreak_property(cp: char) -> u8 {
    let cp = cp as usize;
    if cp < 0x800 {
        LINEBREAK_1_2[cp]
    } else if cp < 0x10000 {
        let child = LINEBREAK_3_ROOT[cp >> 6];
        LINEBREAK_3_CHILD[(child as usize) * 0x40 + (cp & 0x3f)]
    } else {
        let mid = LINEBREAK_4_ROOT[cp >> 12];
        let leaf = LINEBREAK_4_MID[(mid as usize) * 0x40 + ((cp >> 6) & 0x3f)];
        LINEBREAK_4_LEAVES[(leaf as usize) * 0x40 + (cp & 0x3f)]
    }
}

// Return property, length
// May panic if ix doesn't point to a valid character in the string
pub fn linebreak_property_str(s: &str, ix: usize) -> (u8, usize) {
    let b = s.as_bytes()[ix];
    if b < 0x80 {
        (LINEBREAK_1_2[b as usize], 1)
    } else if b < 0xe0 {
        // 2 byte UTF-8 sequences
        let cp = ((b as usize) << 6) + (s.as_bytes()[ix + 1] as usize) - 0x3080;
        (LINEBREAK_1_2[cp], 2)
    } else if b < 0xf0 {
        // 3 byte UTF-8 sequences
        let mid_ix = ((b as usize) << 6) + (s.as_bytes()[ix + 1] as usize) - 0x3880;
        let mid = LINEBREAK_3_ROOT[mid_ix];
        (LINEBREAK_3_CHILD[(mid as usize) * 0x40 + (s.as_bytes()[ix + 2] as usize) - 0x80], 3)
    } else {
        // 4 byte UTF-8 sequences
        let mid_ix = ((b as usize) << 6) + (s.as_bytes()[ix + 1] as usize) - 0x3c80;
        let mid = LINEBREAK_4_ROOT[mid_ix];
        let leaf_ix = ((mid as usize) << 6) + (s.as_bytes()[ix + 2] as usize) - 0x80;
        let leaf = LINEBREAK_4_MID[leaf_ix];
        (LINEBREAK_4_LEAVES[(leaf as usize) * 0x40 + (s.as_bytes()[ix + 3] as usize) - 0x80], 4)
    }
}

/// An iterator which produces line breaks according to the UAX 14 line
/// breaking algorithm. For each break, return a tuple consisting of the offset
/// within the source string and a bool indicating whether it's a hard break.
#[derive(Copy, Clone)]
pub struct LineBreakIterator<'a> {
    s: &'a str,
    ix: usize,
    state: u8,
}

impl<'a> Iterator for LineBreakIterator<'a> {
    type Item = (usize, bool);

    // return break pos and whether it's a hard break
    fn next(&mut self) -> Option<(usize, bool)> {
        loop {
            if self.ix > self.s.len() {
                return None;
            } else if self.ix == self.s.len() {
                // LB3, break at EOT
                self.ix += 1;
                return Some((self.s.len(), true));
            }
            let (lb, len) = linebreak_property_str(self.s, self.ix);
            let i = (self.state as usize) * N_LINEBREAK_CATEGORIES + (lb as usize);
            let new = LINEBREAK_STATE_MACHINE[i];
            //println!("\"{}\"[{}], state {} + lb {} -> {}", &self.s[self.ix..], self.ix, self.state, lb, new);
            let result = self.ix;
            self.ix += len;
            if (new as i8) < 0 {
                // break found
                self.state = new & 0x3f;
                return Some((result, new >= 0xc0));
            } else {
                self.state = new;
            }
        }
    }
}

impl<'a> LineBreakIterator<'a> {
    /// Create a new iterator for the given string slice.
    pub fn new(s: &str) -> LineBreakIterator {
        if s.is_empty() {
            LineBreakIterator {
                s,
                ix: 1,  // LB2, don't break; sot takes priority for empty string
                state: 0,
            }
        } else {
            let (lb, len) = linebreak_property_str(s, 0);
            LineBreakIterator {
                s,
                ix: len,
                state: lb,
            }
        }
    }
}

/// A class (TODO, not right word) useful for computing line breaks in a rope or
/// other non-contiguous string representation. This is a trickier problem than
/// iterating in a string for a few reasons, the trickiest of which is that in
/// the general case, line breaks require an indeterminate amount of look-behind.
///
/// This is something of an "expert-level" interface, and should only be used if
/// the caller is prepared to respect all the invariants. Otherwise, you might
/// get inconsistent breaks depending on start positiona and leaf boundaries.
#[derive(Copy, Clone)]
pub struct LineBreakLeafIter {
    ix: usize,
    state: u8,
}

impl Default for LineBreakLeafIter {
    // A default value. No guarantees on what happens when next() is called
    // on this. Intended to be useful for empty ropes.
    fn default() -> LineBreakLeafIter {
        LineBreakLeafIter {
            ix: 0,
            state: 0,
        }
    }
}

impl LineBreakLeafIter {
    /// Create a new line break iterator suitable for leaves in a rope.
    /// Precondition: ix is at a code point boundary within s.
    pub fn new(s: &str, ix: usize) -> LineBreakLeafIter {
        let (lb, len) = if ix == s.len() {
            (0, 0)
        } else {
            linebreak_property_str(s, ix)
        };
        LineBreakLeafIter {
            ix: ix + len,
            state: lb,
        }
    }

    /// Return break pos and whether it's a hard break. Note: hard break
    /// indication may go away, this may not be useful in actual application.
    /// If end of leaf is found, return leaf's len. This does not indicate
    /// a break, as that requires at least one more codepoint of context.
    /// If it is a break, then subsequent next call will return an offset of 0.
    /// EOT is always a break, so in the EOT case it's up to the caller
    /// to figure that out.
    ///
    /// For consistent results, always supply same `s` until end of leaf is
    /// reached (and initially this should be the same as in the `new` call).
    pub fn next(&mut self, s: &str) -> (usize, bool) {
        loop {
            if self.ix == s.len() {
                self.ix = 0;  // in preparation for next leaf
                return (s.len(), false);
            }
            let (lb, len) = linebreak_property_str(s, self.ix);
            let i = (self.state as usize) * N_LINEBREAK_CATEGORIES + (lb as usize);
            let new = LINEBREAK_STATE_MACHINE[i];
            //println!("\"{}\"[{}], state {} + lb {} -> {}", &s[self.ix..], self.ix, self.state, lb, new);
            let result = self.ix;
            self.ix += len;
            if (new as i8) < 0 {
                // break found
                self.state = new & 0x3f;
                return (result, new >= 0xc0);
            } else {
                self.state = new;
            }
        }
    }
}

fn is_in_asc_list<T: std::cmp::PartialOrd>(c: T, list: &[T], start: usize, end: usize) -> bool {
    if c == list[start] || c == list[end] {
        return true;
    }
    if end - start <= 1 {
        return false;
    }

    let mid = (start + end) / 2;

    if c >= list[mid] {
        return is_in_asc_list(c, &list, mid, end);
    } else {
        return is_in_asc_list(c, &list, start, mid);
    }
}

pub fn is_variation_selector(c: char) -> bool {
    (c >= '\u{FE00}' && c <= '\u{FE0F}') || (c >= '\u{E0100}' && c <= '\u{E01EF}')
}

pub fn is_regional_indicator_symbol(c: char) -> bool {
    c >= '\u{1F1E6}' && c <= '\u{1F1FF}'
}

pub fn is_emoji_modifier(c: char) -> bool {
    c >= '\u{1F3FB}' && c <= '\u{1F3FF}'
}

pub fn is_emoji_combining_enclosing_keycap(c: char) -> bool { c == '\u{20E3}' }

pub fn is_emoji(c: char) -> bool { is_in_asc_list(c, &EMOJI_TABLE, 0, EMOJI_TABLE.len() - 1) }

pub fn is_keycap_base(c: char) -> bool { ('0' <= c && c <= '9') || c == '#' || c == '*' }

pub fn is_emoji_modifier_base(c: char) -> bool {
    is_in_asc_list(c, &EMOJI_MODIFIER_BASE_TABLE, 0, EMOJI_MODIFIER_BASE_TABLE.len() - 1)
}

pub fn is_tag_spec_char(c: char) -> bool { '\u{E0020}' <= c && c <= '\u{E007E}' }

pub fn is_emoji_cancel_tag(c: char) -> bool { c == '\u{E007F}' }

pub fn is_zwj(c: char) -> bool { c == '\u{200D}' }

#[cfg(test)]
mod tests {
    use linebreak_property;
    use linebreak_property_str;
    use LineBreakIterator;

    #[test]
    fn linebreak_prop() {
        assert_eq!( 9, linebreak_property('\u{0001}'));
        assert_eq!( 9, linebreak_property('\u{0003}'));
        assert_eq!( 9, linebreak_property('\u{0004}'));
        assert_eq!( 9, linebreak_property('\u{0008}'));
        assert_eq!(10, linebreak_property('\u{000D}'));
        assert_eq!( 9, linebreak_property('\u{0010}'));
        assert_eq!( 9, linebreak_property('\u{0015}'));
        assert_eq!( 9, linebreak_property('\u{0018}'));
        assert_eq!(22, linebreak_property('\u{002B}'));
        assert_eq!(16, linebreak_property('\u{002C}'));
        assert_eq!(13, linebreak_property('\u{002D}'));
        assert_eq!(27, linebreak_property('\u{002F}'));
        assert_eq!(19, linebreak_property('\u{0030}'));
        assert_eq!(19, linebreak_property('\u{0038}'));
        assert_eq!(19, linebreak_property('\u{0039}'));
        assert_eq!(16, linebreak_property('\u{003B}'));
        assert_eq!( 2, linebreak_property('\u{003E}'));
        assert_eq!(11, linebreak_property('\u{003F}'));
        assert_eq!( 2, linebreak_property('\u{0040}'));
        assert_eq!( 2, linebreak_property('\u{0055}'));
        assert_eq!( 2, linebreak_property('\u{0056}'));
        assert_eq!( 2, linebreak_property('\u{0058}'));
        assert_eq!( 2, linebreak_property('\u{0059}'));
        assert_eq!(20, linebreak_property('\u{005B}'));
        assert_eq!(22, linebreak_property('\u{005C}'));
        assert_eq!( 2, linebreak_property('\u{0062}'));
        assert_eq!( 2, linebreak_property('\u{006C}'));
        assert_eq!( 2, linebreak_property('\u{006D}'));
        assert_eq!( 2, linebreak_property('\u{0071}'));
        assert_eq!( 2, linebreak_property('\u{0074}'));
        assert_eq!( 2, linebreak_property('\u{0075}'));
        assert_eq!( 4, linebreak_property('\u{007C}'));
        assert_eq!( 9, linebreak_property('\u{009D}'));
        assert_eq!( 2, linebreak_property('\u{00D5}'));
        assert_eq!( 2, linebreak_property('\u{00D8}'));
        assert_eq!( 2, linebreak_property('\u{00E9}'));
        assert_eq!( 2, linebreak_property('\u{0120}'));
        assert_eq!( 2, linebreak_property('\u{0121}'));
        assert_eq!( 2, linebreak_property('\u{015C}'));
        assert_eq!( 2, linebreak_property('\u{016C}'));
        assert_eq!( 2, linebreak_property('\u{017E}'));
        assert_eq!( 2, linebreak_property('\u{01B0}'));
        assert_eq!( 2, linebreak_property('\u{0223}'));
        assert_eq!( 2, linebreak_property('\u{028D}'));
        assert_eq!( 2, linebreak_property('\u{02BE}'));
        assert_eq!( 1, linebreak_property('\u{02D0}'));
        assert_eq!( 9, linebreak_property('\u{0337}'));
        assert_eq!( 0, linebreak_property('\u{0380}'));
        assert_eq!( 2, linebreak_property('\u{04AA}'));
        assert_eq!( 2, linebreak_property('\u{04CE}'));
        assert_eq!( 2, linebreak_property('\u{04F1}'));
        assert_eq!( 2, linebreak_property('\u{0567}'));
        assert_eq!( 2, linebreak_property('\u{0580}'));
        assert_eq!( 9, linebreak_property('\u{05A1}'));
        assert_eq!( 9, linebreak_property('\u{05B0}'));
        assert_eq!(38, linebreak_property('\u{05D4}'));
        assert_eq!( 2, linebreak_property('\u{0643}'));
        assert_eq!( 9, linebreak_property('\u{065D}'));
        assert_eq!(19, linebreak_property('\u{066C}'));
        assert_eq!( 2, linebreak_property('\u{066E}'));
        assert_eq!( 2, linebreak_property('\u{068A}'));
        assert_eq!( 2, linebreak_property('\u{0776}'));
        assert_eq!( 2, linebreak_property('\u{07A2}'));
        assert_eq!( 0, linebreak_property('\u{07BB}'));
        assert_eq!(19, linebreak_property('\u{1091}'));
        assert_eq!(19, linebreak_property('\u{1B53}'));
        assert_eq!( 2, linebreak_property('\u{1EEA}'));
        assert_eq!(40, linebreak_property('\u{200D}'));
        assert_eq!(14, linebreak_property('\u{30C7}'));
        assert_eq!(14, linebreak_property('\u{318B}'));
        assert_eq!(14, linebreak_property('\u{3488}'));
        assert_eq!(14, linebreak_property('\u{3B6E}'));
        assert_eq!(14, linebreak_property('\u{475B}'));
        assert_eq!(14, linebreak_property('\u{490B}'));
        assert_eq!(14, linebreak_property('\u{5080}'));
        assert_eq!(14, linebreak_property('\u{7846}'));
        assert_eq!(14, linebreak_property('\u{7F3A}'));
        assert_eq!(14, linebreak_property('\u{8B51}'));
        assert_eq!(14, linebreak_property('\u{920F}'));
        assert_eq!(14, linebreak_property('\u{9731}'));
        assert_eq!(14, linebreak_property('\u{9F3A}'));
        assert_eq!( 2, linebreak_property('\u{ABD2}'));
        assert_eq!(19, linebreak_property('\u{ABF6}'));
        assert_eq!(32, linebreak_property('\u{B2EA}'));
        assert_eq!(32, linebreak_property('\u{B3F5}'));
        assert_eq!(32, linebreak_property('\u{B796}'));
        assert_eq!(32, linebreak_property('\u{B9E8}'));
        assert_eq!(32, linebreak_property('\u{BD42}'));
        assert_eq!(32, linebreak_property('\u{C714}'));
        assert_eq!(32, linebreak_property('\u{CC25}'));
        assert_eq!( 0, linebreak_property('\u{EA59}'));
        assert_eq!( 0, linebreak_property('\u{F6C8}'));
        assert_eq!( 0, linebreak_property('\u{F83C}'));
        assert_eq!( 2, linebreak_property('\u{FC6A}'));
        assert_eq!( 0, linebreak_property('\u{15199}'));
        assert_eq!( 0, linebreak_property('\u{163AC}'));
        assert_eq!( 0, linebreak_property('\u{1EF65}'));
        assert_eq!(14, linebreak_property('\u{235A7}'));
        assert_eq!(14, linebreak_property('\u{2E483}'));
        assert_eq!(14, linebreak_property('\u{2FFFA}'));
        assert_eq!(14, linebreak_property('\u{3613E}'));
        assert_eq!(14, linebreak_property('\u{3799A}'));
        assert_eq!( 0, linebreak_property('\u{4DD35}'));
        assert_eq!( 0, linebreak_property('\u{5858D}'));
        assert_eq!( 0, linebreak_property('\u{585C2}'));
        assert_eq!( 0, linebreak_property('\u{6CF38}'));
        assert_eq!( 0, linebreak_property('\u{7573F}'));
        assert_eq!( 0, linebreak_property('\u{7AABF}'));
        assert_eq!( 0, linebreak_property('\u{87762}'));
        assert_eq!( 0, linebreak_property('\u{90297}'));
        assert_eq!( 0, linebreak_property('\u{9D037}'));
        assert_eq!( 0, linebreak_property('\u{A0E65}'));
        assert_eq!( 0, linebreak_property('\u{B8E7F}'));
        assert_eq!( 0, linebreak_property('\u{BBEA5}'));
        assert_eq!( 0, linebreak_property('\u{BE28C}'));
        assert_eq!( 0, linebreak_property('\u{C1B57}'));
        assert_eq!( 0, linebreak_property('\u{C2011}'));
        assert_eq!( 0, linebreak_property('\u{CBF32}'));
        assert_eq!( 0, linebreak_property('\u{DD9BD}'));
        assert_eq!( 0, linebreak_property('\u{DF4A6}'));
        assert_eq!( 0, linebreak_property('\u{E923D}'));
        assert_eq!( 0, linebreak_property('\u{E94DB}'));
        assert_eq!( 0, linebreak_property('\u{F90AB}'));
        assert_eq!( 0, linebreak_property('\u{100EF6}'));
        assert_eq!( 0, linebreak_property('\u{106487}'));
        assert_eq!( 0, linebreak_property('\u{1064B4}'));
    }

    #[test]
    fn linebreak_prop_str() {
        assert_eq!((9, 1), linebreak_property_str(&"\u{0004}", 0));
        assert_eq!((9, 1), linebreak_property_str(&"\u{0005}", 0));
        assert_eq!((9, 1), linebreak_property_str(&"\u{0008}", 0));
        assert_eq!((4, 1), linebreak_property_str(&"\u{0009}", 0));
        assert_eq!((17, 1), linebreak_property_str(&"\u{000A}", 0));
        assert_eq!((6, 1), linebreak_property_str(&"\u{000C}", 0));
        assert_eq!((9, 1), linebreak_property_str(&"\u{000E}", 0));
        assert_eq!((9, 1), linebreak_property_str(&"\u{0010}", 0));
        assert_eq!((9, 1), linebreak_property_str(&"\u{0013}", 0));
        assert_eq!((9, 1), linebreak_property_str(&"\u{0017}", 0));
        assert_eq!((9, 1), linebreak_property_str(&"\u{001C}", 0));
        assert_eq!((9, 1), linebreak_property_str(&"\u{001D}", 0));
        assert_eq!((9, 1), linebreak_property_str(&"\u{001F}", 0));
        assert_eq!((11, 1), linebreak_property_str(&"\u{0021}", 0));
        assert_eq!((23, 1), linebreak_property_str(&"\u{0027}", 0));
        assert_eq!((22, 1), linebreak_property_str(&"\u{002B}", 0));
        assert_eq!((13, 1), linebreak_property_str(&"\u{002D}", 0));
        assert_eq!((27, 1), linebreak_property_str(&"\u{002F}", 0));
        assert_eq!((2, 1), linebreak_property_str(&"\u{003C}", 0));
        assert_eq!((2, 1), linebreak_property_str(&"\u{0043}", 0));
        assert_eq!((2, 1), linebreak_property_str(&"\u{004B}", 0));
        assert_eq!((36, 1), linebreak_property_str(&"\u{005D}", 0));
        assert_eq!((2, 1), linebreak_property_str(&"\u{0060}", 0));
        assert_eq!((2, 1), linebreak_property_str(&"\u{0065}", 0));
        assert_eq!((2, 1), linebreak_property_str(&"\u{0066}", 0));
        assert_eq!((2, 1), linebreak_property_str(&"\u{0068}", 0));
        assert_eq!((2, 1), linebreak_property_str(&"\u{0069}", 0));
        assert_eq!((2, 1), linebreak_property_str(&"\u{006C}", 0));
        assert_eq!((2, 1), linebreak_property_str(&"\u{006D}", 0));
        assert_eq!((2, 1), linebreak_property_str(&"\u{0077}", 0));
        assert_eq!((2, 1), linebreak_property_str(&"\u{0079}", 0));
        assert_eq!((4, 1), linebreak_property_str(&"\u{007C}", 0));
        assert_eq!((9, 2), linebreak_property_str(&"\u{008D}", 0));
        assert_eq!((1, 2), linebreak_property_str(&"\u{00D7}", 0));
        assert_eq!((2, 2), linebreak_property_str(&"\u{015C}", 0));
        assert_eq!((2, 2), linebreak_property_str(&"\u{01B5}", 0));
        assert_eq!((2, 2), linebreak_property_str(&"\u{0216}", 0));
        assert_eq!((2, 2), linebreak_property_str(&"\u{0234}", 0));
        assert_eq!((2, 2), linebreak_property_str(&"\u{026E}", 0));
        assert_eq!((2, 2), linebreak_property_str(&"\u{027C}", 0));
        assert_eq!((2, 2), linebreak_property_str(&"\u{02BB}", 0));
        assert_eq!((9, 2), linebreak_property_str(&"\u{0313}", 0));
        assert_eq!((9, 2), linebreak_property_str(&"\u{0343}", 0));
        assert_eq!((9, 2), linebreak_property_str(&"\u{034A}", 0));
        assert_eq!((9, 2), linebreak_property_str(&"\u{0358}", 0));
        assert_eq!((0, 2), linebreak_property_str(&"\u{0378}", 0));
        assert_eq!((2, 2), linebreak_property_str(&"\u{038C}", 0));
        assert_eq!((2, 2), linebreak_property_str(&"\u{03A4}", 0));
        assert_eq!((2, 2), linebreak_property_str(&"\u{03AC}", 0));
        assert_eq!((2, 2), linebreak_property_str(&"\u{041F}", 0));
        assert_eq!((2, 2), linebreak_property_str(&"\u{049A}", 0));
        assert_eq!((2, 2), linebreak_property_str(&"\u{04B4}", 0));
        assert_eq!((2, 2), linebreak_property_str(&"\u{04C6}", 0));
        assert_eq!((2, 2), linebreak_property_str(&"\u{0535}", 0));
        assert_eq!((9, 2), linebreak_property_str(&"\u{05B1}", 0));
        assert_eq!((0, 2), linebreak_property_str(&"\u{05FF}", 0));
        assert_eq!((9, 2), linebreak_property_str(&"\u{065D}", 0));
        assert_eq!((2, 2), linebreak_property_str(&"\u{067E}", 0));
        assert_eq!((19, 2), linebreak_property_str(&"\u{06F5}", 0));
        assert_eq!((19, 2), linebreak_property_str(&"\u{06F6}", 0));
        assert_eq!((9, 2), linebreak_property_str(&"\u{0735}", 0));
        assert_eq!((2, 2), linebreak_property_str(&"\u{074D}", 0));
        assert_eq!((9, 2), linebreak_property_str(&"\u{07A6}", 0));
        assert_eq!((0, 2), linebreak_property_str(&"\u{07B9}", 0));
        assert_eq!((2, 3), linebreak_property_str(&"\u{131F}", 0));
        assert_eq!((40, 3), linebreak_property_str(&"\u{200D}", 0));
        assert_eq!((2, 3), linebreak_property_str(&"\u{25DA}", 0));
        assert_eq!((2, 3), linebreak_property_str(&"\u{2C01}", 0));
        assert_eq!((14, 3), linebreak_property_str(&"\u{2EE5}", 0));
        assert_eq!((14, 3), linebreak_property_str(&"\u{4207}", 0));
        assert_eq!((14, 3), linebreak_property_str(&"\u{4824}", 0));
        assert_eq!((14, 3), linebreak_property_str(&"\u{491A}", 0));
        assert_eq!((14, 3), linebreak_property_str(&"\u{4C20}", 0));
        assert_eq!((14, 3), linebreak_property_str(&"\u{4D6A}", 0));
        assert_eq!((14, 3), linebreak_property_str(&"\u{50EB}", 0));
        assert_eq!((14, 3), linebreak_property_str(&"\u{521B}", 0));
        assert_eq!((14, 3), linebreak_property_str(&"\u{5979}", 0));
        assert_eq!((14, 3), linebreak_property_str(&"\u{5F9B}", 0));
        assert_eq!((14, 3), linebreak_property_str(&"\u{65AB}", 0));
        assert_eq!((14, 3), linebreak_property_str(&"\u{6B1F}", 0));
        assert_eq!((14, 3), linebreak_property_str(&"\u{7169}", 0));
        assert_eq!((14, 3), linebreak_property_str(&"\u{87CA}", 0));
        assert_eq!((14, 3), linebreak_property_str(&"\u{87FF}", 0));
        assert_eq!((14, 3), linebreak_property_str(&"\u{8A91}", 0));
        assert_eq!((14, 3), linebreak_property_str(&"\u{943A}", 0));
        assert_eq!((14, 3), linebreak_property_str(&"\u{9512}", 0));
        assert_eq!((14, 3), linebreak_property_str(&"\u{9D66}", 0));
        assert_eq!((9, 3), linebreak_property_str(&"\u{A928}", 0));
        assert_eq!((24, 3), linebreak_property_str(&"\u{AA7E}", 0));
        assert_eq!((2, 3), linebreak_property_str(&"\u{AAEA}", 0));
        assert_eq!((0, 3), linebreak_property_str(&"\u{AB66}", 0));
        assert_eq!((32, 3), linebreak_property_str(&"\u{B9FC}", 0));
        assert_eq!((32, 3), linebreak_property_str(&"\u{CD89}", 0));
        assert_eq!((32, 3), linebreak_property_str(&"\u{CDB2}", 0));
        assert_eq!((0, 3), linebreak_property_str(&"\u{F71D}", 0));
        assert_eq!((14, 3), linebreak_property_str(&"\u{F9DF}", 0));
        assert_eq!((2, 3), linebreak_property_str(&"\u{FEC3}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{13CC5}", 0));
        assert_eq!((2, 4), linebreak_property_str(&"\u{1D945}", 0));
        assert_eq!((41, 4), linebreak_property_str(&"\u{1F3C3}", 0));
        assert_eq!((42, 4), linebreak_property_str(&"\u{1F3FB}", 0));
        assert_eq!((14, 4), linebreak_property_str(&"\u{2BDCD}", 0));
        assert_eq!((14, 4), linebreak_property_str(&"\u{3898E}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{45C35}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{4EC30}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{58EE2}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{5E3E8}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{5FB7D}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{6A564}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{6C591}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{6CA82}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{83839}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{88F47}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{91CA0}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{95644}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{AC335}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{AE8BF}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{B282B}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{B4CFC}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{BBED0}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{CCC89}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{D40EB}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{D65F5}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{D8E0B}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{DF93A}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{E4E2C}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{F7935}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{F9DFF}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{1094B7}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{10C782}", 0));
        assert_eq!((0, 4), linebreak_property_str(&"\u{10E4D5}", 0));
    }

    #[test]
    fn lb_iter_simple() {
        assert_eq!(vec![(6, false), (11, true)],
            LineBreakIterator::new("hello world").collect::<Vec<_>>());

        // LB7, LB18
        assert_eq!(vec![(3, false), (4, true)],
            LineBreakIterator::new("a  b").collect::<Vec<_>>());

        // LB5
        assert_eq!(vec![(2, true), (3, true)],
            LineBreakIterator::new("a\nb").collect::<Vec<_>>());
        assert_eq!(vec![(2, true), (4, true)],
            LineBreakIterator::new("\r\n\r\n").collect::<Vec<_>>());

        // LB8a
        assert_eq!(vec![(7, true)],
            LineBreakIterator::new("\u{200D}\u{1F3FB}").collect::<Vec<_>>());

        // LB10 combining mark after space
        assert_eq!(vec![(2, false), (4, true)],
            LineBreakIterator::new("a \u{301}").collect::<Vec<_>>());

        // LB15
        assert_eq!(vec![(3, true)],
            LineBreakIterator::new("\" [").collect::<Vec<_>>());

        // LB17
        assert_eq!(vec![(2, false), (10, false), (11, true)],
            LineBreakIterator::new("a \u{2014} \u{2014} c").collect::<Vec<_>>());

        // LB18
        assert_eq!(vec![(2, false), (6, false), (7, true)],
            LineBreakIterator::new("a \"b\" c").collect::<Vec<_>>());

        // LB21
        assert_eq!(vec![(2, false), (3, true)],
            LineBreakIterator::new("a-b").collect::<Vec<_>>());

        // LB21a
        assert_eq!(vec![(5, true)],
            LineBreakIterator::new("\u{05D0}-\u{05D0}").collect::<Vec<_>>());

        // LB23a
        assert_eq!(vec![(6, true)],
            LineBreakIterator::new("$\u{1F3FB}%").collect::<Vec<_>>());

        // LB30b
        assert_eq!(vec![(8, true)],
            LineBreakIterator::new("\u{1F466}\u{1F3FB}").collect::<Vec<_>>());

        // LB31
        assert_eq!(vec![(8, false), (16, true)],
            LineBreakIterator::new("\u{1F1E6}\u{1F1E6}\u{1F1E6}\u{1F1E6}").collect::<Vec<_>>());
    }
}
