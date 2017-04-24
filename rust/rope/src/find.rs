// Copyright 2017 Google Inc. All rights reserved.
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

//! Implementation of string finding in ropes.

use rope::RopeInfo;
use tree::Cursor;

use memchr::{memchr, memchr2, memchr3};

pub enum FindResult {
    Found(usize),
    NotFound,
    TryAgain,
}

/// Case-sensitive find (just does byte comparison, no Unicode logic).
///
/// Can panic if `pat` is empty.
pub fn find(cursor: &mut Cursor<RopeInfo>, pat: &str) -> Option<usize> {
    loop {
        match find_step(cursor, pat) {
            FindResult::Found(pos) => return Some(pos),
            FindResult::NotFound => return None,
            _ => (),
        }
    }
}

/// Case-insensitive find, using Rust's `to_lowercase` as the basis
/// for comparison.
///
/// Can panic if `pat` is empty.

// Note: one limitation of this method is that 'k' as the first
// character in `pat` will not match against U+212A (kelvin). Seems
// low-frequency enough, but should be fixed eventually.
pub fn find_casei(cursor: &mut Cursor<RopeInfo>, pat: &str) -> Option<usize> {
    let pat_lower = pat.to_lowercase();
    let b = pat_lower.as_bytes()[0];
    if b == b'i' {
        loop {
            match find_ascii_i_step(cursor, &pat_lower) {
                FindResult::Found(pos) => return Some(pos),
                FindResult::NotFound => return None,
                _ => (),
            }
        }
    } else if b >= b'a' && b <= b'z' {
        loop {
            match find_ascii_alpha_step(cursor, &pat_lower) {
                FindResult::Found(pos) => return Some(pos),
                FindResult::NotFound => return None,
                _ => (),
            }
        }
    } else if b < 0x80 {
        loop {
            match find_ascii_step(cursor, &pat_lower) {
                FindResult::Found(pos) => return Some(pos),
                FindResult::NotFound => return None,
                _ => (),
            }
        }
    } else {
        loop {
            match find_casei_step(cursor, &pat_lower) {
                FindResult::Found(pos) => return Some(pos),
                FindResult::NotFound => return None,
                _ => (),
            }
        }
    }
}

/// Case-sensitive find (just does byte comparison, no Unicode logic).
///
/// This method is like [`find`] but does a bounded amount of work.
fn find_step(cursor: &mut Cursor<RopeInfo>, pat: &str) -> FindResult {
    let orig_pos = cursor.pos();
    if let Some((leaf, pos_in_leaf)) = cursor.get_leaf() {
        let b = pat.as_bytes()[0];
        if let Some(off) = memchr(b, &leaf.as_bytes()[pos_in_leaf..]) {
            let candidate_pos = orig_pos + off;
            cursor.set(candidate_pos);
            if compare_cursor_str(cursor, pat) {
                return FindResult::Found(candidate_pos);
            } else {
                cursor.set(candidate_pos);
                // TODO: can optimize this, the number of bytes to advance is
                // a function of b.
                let _ = cursor.next_codepoint();
            }
        } else {
            let _ = cursor.next_leaf();
        }
        FindResult::TryAgain
    } else {
        FindResult::NotFound
    }
}

/// Case-insensitive find.
///
/// Variant in which first character in `pat` is `[a-z]`.
fn find_ascii_alpha_step(cursor: &mut Cursor<RopeInfo>, pat: &str) -> FindResult {
    let orig_pos = cursor.pos();
    if let Some((leaf, pos_in_leaf)) = cursor.get_leaf() {
        let b = pat.as_bytes()[0];
        if let Some(off) = memchr2(b, b - 0x20, &leaf.as_bytes()[pos_in_leaf..]) {
            let candidate_pos = orig_pos + off;
            cursor.set(candidate_pos);
            if compare_cursor_str_casei(cursor, pat) {
                return FindResult::Found(candidate_pos);
            } else {
                // Advance to pos after first character match.
                cursor.set(candidate_pos + 1);
            }
        } else {
            let _ = cursor.next_leaf();
        }
        FindResult::TryAgain
    } else {
        FindResult::NotFound
    }
}

/// Case-insensitive find.
///
/// Variant in which first character in `pat` is `i`.
fn find_ascii_i_step(cursor: &mut Cursor<RopeInfo>, pat: &str) -> FindResult {
    let orig_pos = cursor.pos();
    if let Some((leaf, pos_in_leaf)) = cursor.get_leaf() {
        // 0xC4 is first byte of 'İ'
        if let Some(off) = memchr3(b'i', b'I', 0xC4, &leaf.as_bytes()[pos_in_leaf..]) {
            let candidate_pos = orig_pos + off;
            cursor.set(candidate_pos);
            if compare_cursor_str_casei(cursor, pat) {
                return FindResult::Found(candidate_pos);
            } else {
                // Advance to pos after first character match.
                cursor.set(candidate_pos + 1);
            }
        } else {
            let _ = cursor.next_leaf();
        }
        FindResult::TryAgain
    } else {
        FindResult::NotFound
    }
}

/// Case-insensitive find.
///
/// Variant in which first character in `pat` is ASCII but not `[a-z]`.
fn find_ascii_step(cursor: &mut Cursor<RopeInfo>, pat: &str) -> FindResult {
    let orig_pos = cursor.pos();
    if let Some((leaf, pos_in_leaf)) = cursor.get_leaf() {
        let b = pat.as_bytes()[0];
        if let Some(off) = memchr(b, &leaf.as_bytes()[pos_in_leaf..]) {
            let candidate_pos = orig_pos + off;
            cursor.set(candidate_pos);
            if compare_cursor_str_casei(cursor, pat) {
                return FindResult::Found(candidate_pos);
            } else {
                // Advance to pos after first character match.
                cursor.set(candidate_pos + 1);
            }
        } else {
            let _ = cursor.next_leaf();
        }
        FindResult::TryAgain
    } else {
        FindResult::NotFound
    }
}

/// Case-insensitive find.
///
/// Most general variant; this will match cases such as İ and i, but is really
/// slow. 
fn find_casei_step(cursor: &mut Cursor<RopeInfo>, pat: &str) -> FindResult {
    let orig_pos = cursor.pos();
    if let Some((_leaf, _pos_in_leaf)) = cursor.get_leaf() {
        if compare_cursor_str_casei(cursor, pat) {
            return FindResult::Found(orig_pos);
        } else {
            cursor.set(orig_pos);
            // TODO: can optimize this, the number of bytes to advance is
            // a function of first byte in pat.
            let _ = cursor.next_codepoint();
        }
        FindResult::TryAgain
    } else {
        FindResult::NotFound
    }
}

/// Compare whether the substring beginning at the current cursor location
/// is equal to the provided string. Leaves the cursor at an indeterminate
/// position on failure, but the end of the string on success.

// Note: this could be rewritten in terms of memory chunks
fn compare_cursor_str(cursor: &mut Cursor<RopeInfo>, pat: &str) -> bool {
    for c in pat.chars() {
        if let Some(rope_c) = cursor.next_codepoint() {
            if rope_c != c { return false; }
        } else {
            // end of string before pattern is complete
            return false;
        }
    }
    true
}

/// Like `compare_cursor_str` but case invariant (using to_lowercase() to
/// normalize both strings before comparison).
fn compare_cursor_str_casei(cursor: &mut Cursor<RopeInfo>, pat: &str) -> bool {
    let mut pat_iter = pat.chars();
    let mut c = pat_iter.next().unwrap();
    loop {
        if let Some(rope_c) = cursor.next_codepoint() {
            for lc_c in rope_c.to_lowercase() {
                if c != lc_c {
                    return false;
                }
                if let Some(next_c) = pat_iter.next() {
                    c = next_c;
                } else {
                    return true;
                }
            }
        } else {
            // end of string before pattern is complete
            return false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree::Cursor;
    use rope::Rope;

    #[test]
    fn find_small() {
        let a = Rope::from("Löwe 老虎 Léopard");
        let mut c = Cursor::new(&a, 0);
        assert_eq!(find(&mut c, "L"), Some(0));
        assert_eq!(find(&mut c, "L"), Some(13));
        assert_eq!(find(&mut c, "L"), None);
        c.set(0);
        assert_eq!(find(&mut c, "Léopard"), Some(13));
        assert_eq!(find(&mut c, "Léopard"), None);
        c.set(0);
        assert_eq!(find(&mut c, "老虎"), Some(6));
        assert_eq!(find(&mut c, "老虎"), None);
        c.set(0);
        assert_eq!(find(&mut c, "Tiger"), None);
    }

    #[test]
    fn find_medium() {
        let mut s = String::new();
        for _ in 0..4000 {
            s.push('x');
        }
        s.push_str("Löwe 老虎 Léopard");
        let a = Rope::from(&s);
        let mut c = Cursor::new(&a, 0);
        assert_eq!(find(&mut c, "L"), Some(4000));
        assert_eq!(find(&mut c, "L"), Some(4013));
        assert_eq!(find(&mut c, "L"), None);
        c.set(0);
        assert_eq!(find(&mut c, "Léopard"), Some(4013));
        assert_eq!(find(&mut c, "Léopard"), None);
        c.set(0);
        assert_eq!(find(&mut c, "老虎"), Some(4006));
        assert_eq!(find(&mut c, "老虎"), None);
        c.set(0);
        assert_eq!(find(&mut c, "Tiger"), None);
    }

    #[test]
    fn find_casei_small() {
        let a = Rope::from("Löwe 老虎 Léopard");
        let mut c = Cursor::new(&a, 0);
        assert_eq!(find_casei(&mut c, "l"), Some(0));
        assert_eq!(find_casei(&mut c, "l"), Some(13));
        assert_eq!(find_casei(&mut c, "l"), None);
        c.set(0);
        assert_eq!(find_casei(&mut c, "léopard"), Some(13));
        assert_eq!(find_casei(&mut c, "léopard"), None);
        c.set(0);
        assert_eq!(find_casei(&mut c, "LÉOPARD"), Some(13));
        assert_eq!(find_casei(&mut c, "LÉOPARD"), None);
        c.set(0);
        assert_eq!(find_casei(&mut c, "老虎"), Some(6));
        assert_eq!(find_casei(&mut c, "老虎"), None);
        c.set(0);
        assert_eq!(find_casei(&mut c, "Tiger"), None);
    }

    #[test]
    fn find_casei_ascii_nonalpha() {
        let a = Rope::from("![cfg(test)]");
        let mut c = Cursor::new(&a, 0);
        assert_eq!(find_casei(&mut c, "(test)"), Some(5));
        c.set(0);
        assert_eq!(find_casei(&mut c, "(TEST)"), Some(5));
    }

    #[test]
    fn find_casei_special() {
        let a = Rope::from("İ");
        let mut c = Cursor::new(&a, 0);
        assert_eq!(find_casei(&mut c, "i̇"), Some(0));

        let a = Rope::from("i̇");
        let mut c = Cursor::new(&a, 0);
        assert_eq!(find_casei(&mut c, "İ"), Some(0));
    }
}
