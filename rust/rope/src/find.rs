// Copyright 2017 The xi-editor Authors.
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

use std::cmp::min;

use memchr::{memchr, memchr2, memchr3};

use crate::rope::BaseMetric;
use crate::rope::LinesRaw;
use crate::rope::RopeInfo;
use crate::tree::Cursor;
use regex::Regex;
use std::borrow::Cow;
use std::str;

/// The result of a [`find`][find] operation.
///
/// [find]: fn.find.html
pub enum FindResult {
    /// The pattern was found at this position.
    Found(usize),
    /// The pattern was not found.
    NotFound,
    /// The cursor has been advanced by some amount. The pattern is not
    /// found before the new cursor, but may be at or beyond it.
    TryAgain,
}

/// A policy for case matching. There may be more choices in the future (for
/// example, an even more forgiving mode that ignores accents, or possibly
/// handling Unicode normalization).
#[derive(Clone, Copy, PartialEq)]
pub enum CaseMatching {
    /// Require an exact codepoint-for-codepoint match (implies case sensitivity).
    Exact,
    /// Case insensitive match. Guaranteed to work for the ASCII case, and
    /// reasonably well otherwise (it is currently defined in terms of the
    /// `to_lowercase` methods in the Rust standard library).
    CaseInsensitive,
}

/// Finds a pattern string in the rope referenced by the cursor, starting at
/// the current location of the cursor (and finding the first match). Both
/// case sensitive and case insensitive matching is provided, controlled by
/// the `cm` parameter. The `regex` parameter controls whether the query
/// should be considered as a regular expression.
///
/// On success, the cursor is updated to immediately follow the found string.
/// On failure, the cursor's position is indeterminate.
///
/// Can panic if `pat` is empty.
pub fn find(
    cursor: &mut Cursor<RopeInfo>,
    lines: &mut LinesRaw,
    cm: CaseMatching,
    pat: &str,
    regex: Option<&Regex>,
) -> Option<usize> {
    match find_progress(cursor, lines, cm, pat, usize::max_value(), regex) {
        FindResult::Found(start) => Some(start),
        FindResult::NotFound => None,
        FindResult::TryAgain => unreachable!("find_progress got stuck"),
    }
}

/// A variant of [`find`][find] that makes a bounded amount of progress, then either
/// returns or suspends (returning `TryAgain`).
///
/// The `num_steps` parameter controls the number of "steps" processed per
/// call. The unit of "step" is not formally defined but is typically
/// scanning one leaf (using a memchr-like scan) or testing one candidate
/// when scanning produces a result. It should be empirically tuned for a
/// balance between overhead and impact on interactive performance, but the
/// exact value is probably not critical.
///
/// [find]: fn.find.html
pub fn find_progress(
    cursor: &mut Cursor<RopeInfo>,
    lines: &mut LinesRaw,
    cm: CaseMatching,
    pat: &str,
    num_steps: usize,
    regex: Option<&Regex>,
) -> FindResult {
    // empty search string
    if pat.is_empty() {
        return FindResult::NotFound;
    }

    match regex {
        Some(r) => find_progress_iter(
            cursor,
            lines,
            pat,
            |_| Some(0),
            |cursor, lines, pat| compare_cursor_regex(cursor, lines, pat, r),
            num_steps,
        ),
        None => {
            match cm {
                CaseMatching::Exact => {
                    let b = pat.as_bytes()[0];
                    let scanner = |s: &str| memchr(b, s.as_bytes());
                    let matcher = compare_cursor_str;
                    find_progress_iter(cursor, lines, pat, scanner, matcher, num_steps)
                }
                CaseMatching::CaseInsensitive => {
                    let pat_lower = pat.to_lowercase();
                    let b = pat_lower.as_bytes()[0];
                    let matcher = compare_cursor_str_casei;
                    if b == b'i' {
                        // 0xC4 is first utf-8 byte of 'İ'
                        let scanner = |s: &str| memchr3(b'i', b'I', 0xC4, s.as_bytes());
                        find_progress_iter(cursor, lines, &pat_lower, scanner, matcher, num_steps)
                    } else if b == b'k' {
                        // 0xE2 is first utf-8 byte of u+212A (kelvin sign)
                        let scanner = |s: &str| memchr3(b'k', b'K', 0xE2, s.as_bytes());
                        find_progress_iter(cursor, lines, &pat_lower, scanner, matcher, num_steps)
                    } else if (b'a'..=b'z').contains(&b) {
                        let scanner = |s: &str| memchr2(b, b - 0x20, s.as_bytes());
                        find_progress_iter(cursor, lines, &pat_lower, scanner, matcher, num_steps)
                    } else if b < 0x80 {
                        let scanner = |s: &str| memchr(b, s.as_bytes());
                        find_progress_iter(cursor, lines, &pat_lower, scanner, matcher, num_steps)
                    } else {
                        let c = pat.chars().next().unwrap();
                        let scanner = |s: &str| scan_lowercase(c, s);
                        find_progress_iter(cursor, lines, &pat_lower, scanner, matcher, num_steps)
                    }
                }
            }
        }
    }
}

// Run the core repeatedly until there is a result, up to a certain number of steps.
fn find_progress_iter(
    cursor: &mut Cursor<RopeInfo>,
    lines: &mut LinesRaw,
    pat: &str,
    scanner: impl Fn(&str) -> Option<usize>,
    matcher: impl Fn(&mut Cursor<RopeInfo>, &mut LinesRaw, &str) -> Option<usize>,
    num_steps: usize,
) -> FindResult {
    for _ in 0..num_steps {
        match find_core(cursor, lines, pat, &scanner, &matcher) {
            FindResult::TryAgain => (),
            result => return result,
        }
    }
    FindResult::TryAgain
}

// The core of the find algorithm. It takes a "scanner", which quickly
// scans through a single leaf searching for some prefix of the pattern,
// then a "matcher" which confirms that such a candidate actually matches
// in the full rope.
fn find_core(
    cursor: &mut Cursor<RopeInfo>,
    lines: &mut LinesRaw,
    pat: &str,
    scanner: impl Fn(&str) -> Option<usize>,
    matcher: impl Fn(&mut Cursor<RopeInfo>, &mut LinesRaw, &str) -> Option<usize>,
) -> FindResult {
    let orig_pos = cursor.pos();

    // if cursor reached the end of the text then no match has been found
    if orig_pos == cursor.total_len() {
        return FindResult::NotFound;
    }

    if let Some((leaf, pos_in_leaf)) = cursor.get_leaf() {
        if let Some(off) = scanner(&leaf[pos_in_leaf..]) {
            let candidate_pos = orig_pos + off;
            cursor.set(candidate_pos);
            if let Some(actual_pos) = matcher(cursor, lines, pat) {
                return FindResult::Found(actual_pos);
            }
        } else {
            let _ = cursor.next_leaf();
        }

        FindResult::TryAgain
    } else {
        FindResult::NotFound
    }
}

/// Compare whether the substring beginning at the current cursor location
/// is equal to the provided string. Leaves the cursor at an indeterminate
/// position on failure, but the end of the string on success. Returns the
/// start position of the match.
pub fn compare_cursor_str(
    cursor: &mut Cursor<RopeInfo>,
    _lines: &mut LinesRaw,
    mut pat: &str,
) -> Option<usize> {
    let start_position = cursor.pos();
    if pat.is_empty() {
        return Some(start_position);
    }
    let success_pos = cursor.pos() + pat.len();
    while let Some((leaf, pos_in_leaf)) = cursor.get_leaf() {
        let n = min(pat.len(), leaf.len() - pos_in_leaf);
        if leaf.as_bytes()[pos_in_leaf..pos_in_leaf + n] != pat.as_bytes()[..n] {
            cursor.set(start_position);
            cursor.next::<BaseMetric>();
            return None;
        }
        pat = &pat[n..];
        if pat.is_empty() {
            cursor.set(success_pos);
            return Some(start_position);
        }
        let _ = cursor.next_leaf();
    }
    cursor.set(start_position);
    cursor.next::<BaseMetric>();
    None
}

/// Like `compare_cursor_str` but case invariant (using to_lowercase() to
/// normalize both strings before comparison). Returns the start position
/// of the match.
pub fn compare_cursor_str_casei(
    cursor: &mut Cursor<RopeInfo>,
    _lines: &mut LinesRaw,
    pat: &str,
) -> Option<usize> {
    let start_position = cursor.pos();
    let mut pat_iter = pat.chars();
    let mut c = pat_iter.next().unwrap();
    loop {
        if let Some(rope_c) = cursor.next_codepoint() {
            for lc_c in rope_c.to_lowercase() {
                if c != lc_c {
                    cursor.set(start_position);
                    cursor.next::<BaseMetric>();
                    return None;
                }
                if let Some(next_c) = pat_iter.next() {
                    c = next_c;
                } else {
                    return Some(start_position);
                }
            }
        } else {
            // end of string before pattern is complete
            cursor.set(start_position);
            cursor.next::<BaseMetric>();
            return None;
        }
    }
}

/// Compare whether the substring beginning at the cursor location matches
/// the provided regular expression. The substring begins at the beginning
/// of the start of the line.
/// If the regular expression can match multiple lines then the entire text
/// is consumed and matched against the regular expression. Otherwise only
/// the current line is matched. Returns the start position of the match.
pub fn compare_cursor_regex(
    cursor: &mut Cursor<RopeInfo>,
    lines: &mut LinesRaw,
    pat: &str,
    regex: &Regex,
) -> Option<usize> {
    let orig_position = cursor.pos();

    if pat.is_empty() {
        return Some(orig_position);
    }

    let text: Cow<str>;

    if is_multiline_regex(pat) {
        // consume all of the text if regex is multi line matching
        text = Cow::Owned(lines.collect());
    } else {
        match lines.next() {
            Some(line) => text = line,
            _ => return None,
        }
    }

    // match regex against text
    match regex.find(&text) {
        Some(mat) => {
            // calculate start position based on where the match starts
            let start_position = orig_position + mat.start();

            // update cursor and set to end of match
            let end_position = orig_position + mat.end();
            cursor.set(end_position);
            Some(start_position)
        }
        None => {
            cursor.set(orig_position + text.len());
            None
        }
    }
}

/// Checks if a regular expression can match multiple lines.
pub fn is_multiline_regex(regex: &str) -> bool {
    // regex characters that match line breaks
    // todo: currently multiline mode is ignored
    let multiline_indicators = vec![r"\n", r"\r", r"[[:space:]]"];

    multiline_indicators.iter().any(|&i| regex.contains(i))
}

/// Scan for a codepoint that, after conversion to lowercase, matches the probe.
fn scan_lowercase(probe: char, s: &str) -> Option<usize> {
    for (i, c) in s.char_indices() {
        if c.to_lowercase().next().unwrap() == probe {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::CaseMatching::{CaseInsensitive, Exact};
    use super::*;
    use crate::rope::Rope;
    use crate::tree::Cursor;
    use regex::RegexBuilder;

    const REGEX_SIZE_LIMIT: usize = 1000000;

    #[test]
    fn find_small() {
        let a = Rope::from("Löwe 老虎 Léopard");
        let mut c = Cursor::new(&a, 0);
        let mut raw_lines = a.lines_raw(..);
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "L", None), Some(0));
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "L", None), Some(13));
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "L", None), None);
        c.set(0);
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "Léopard", None), Some(13));
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "Léopard", None), None);
        c.set(0);
        // Note: these two characters both start with 0xE8 in utf-8
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "老虎", None), Some(6));
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "老虎", None), None);
        c.set(0);
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "虎", None), Some(9));
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "虎", None), None);
        c.set(0);
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "Tiger", None), None);
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
        let mut raw_lines = a.lines_raw(0..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "L", None), Some(4000));
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "L", None), Some(4013));
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "L", None), None);
        c.set(0);
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "Léopard", None), Some(4013));
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "Léopard", None), None);
        c.set(0);
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "老虎", None), Some(4006));
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "老虎", None), None);
        c.set(0);
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "虎", None), Some(4009));
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "虎", None), None);
        c.set(0);
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "Tiger", None), None);
    }

    #[test]
    fn find_casei_small() {
        let a = Rope::from("Löwe 老虎 Léopard");
        let mut c = Cursor::new(&a, 0);
        let mut raw_lines = a.lines_raw(0..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "l", None), Some(0));
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "l", None), Some(13));
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "l", None), None);
        c.set(0);
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "léopard", None), Some(13));
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "léopard", None), None);
        c.set(0);
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "LÉOPARD", None), Some(13));
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "LÉOPARD", None), None);
        c.set(0);
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "老虎", None), Some(6));
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "老虎", None), None);
        c.set(0);
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "Tiger", None), None);
    }

    #[test]
    fn find_casei_ascii_nonalpha() {
        let a = Rope::from("![cfg(test)]");
        let mut c = Cursor::new(&a, 0);
        let mut raw_lines = a.lines_raw(0..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "(test)", None), Some(5));
        c.set(0);
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "(TEST)", None), Some(5));
    }

    #[test]
    fn find_casei_special() {
        let a = Rope::from("İ");
        let mut c = Cursor::new(&a, 0);
        let mut raw_lines = a.lines_raw(0..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "i̇", None), Some(0));

        let a = Rope::from("i̇");
        let mut c = Cursor::new(&a, 0);
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "İ", None), Some(0));

        let a = Rope::from("\u{212A}");
        let mut c = Cursor::new(&a, 0);
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "k", None), Some(0));

        let a = Rope::from("k");
        let mut c = Cursor::new(&a, 0);
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "\u{212A}", None), Some(0));
    }

    #[test]
    fn find_casei_0xc4() {
        let a = Rope::from("\u{0100}I");
        let mut c = Cursor::new(&a, 0);
        let mut raw_lines = a.lines_raw(0..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "i", None), Some(2));
    }

    #[test]
    fn find_regex_small_casei() {
        let a = Rope::from("Löwe 老虎 Léopard\nSecond line");
        let mut c = Cursor::new(&a, 0);
        let mut raw_lines = a.lines_raw(0..a.len());
        let regex =
            RegexBuilder::new("L").size_limit(REGEX_SIZE_LIMIT).case_insensitive(true).build().ok();
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "L", regex.as_ref()), Some(0));
        raw_lines = a.lines_raw(c.pos()..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "L", regex.as_ref()), Some(13));
        raw_lines = a.lines_raw(c.pos()..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "L", regex.as_ref()), Some(29));
        c.set(0);
        let regex = RegexBuilder::new("Léopard")
            .size_limit(REGEX_SIZE_LIMIT)
            .case_insensitive(true)
            .build()
            .ok();
        let mut raw_lines = a.lines_raw(0..a.len());
        assert_eq!(
            find(&mut c, &mut raw_lines, CaseInsensitive, "Léopard", regex.as_ref()),
            Some(13)
        );
        raw_lines = a.lines_raw(c.pos()..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "Léopard", regex.as_ref()), None);
        c.set(0);
        let mut raw_lines = a.lines_raw(0..a.len());
        let regex = RegexBuilder::new("老虎")
            .size_limit(REGEX_SIZE_LIMIT)
            .case_insensitive(true)
            .build()
            .ok();
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "老虎", regex.as_ref()), Some(6));
        raw_lines = a.lines_raw(c.pos()..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "老虎", regex.as_ref()), None);
        c.set(0);
        let regex = RegexBuilder::new("Tiger")
            .size_limit(REGEX_SIZE_LIMIT)
            .case_insensitive(true)
            .build()
            .ok();
        let mut raw_lines = a.lines_raw(0..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "Tiger", regex.as_ref()), None);
        c.set(0);
        let regex =
            RegexBuilder::new(".").size_limit(REGEX_SIZE_LIMIT).case_insensitive(true).build().ok();
        let mut raw_lines = a.lines_raw(0..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, ".", regex.as_ref()), Some(0));
        raw_lines = a.lines_raw(c.pos()..a.len());
        let regex = RegexBuilder::new("\\s")
            .size_limit(REGEX_SIZE_LIMIT)
            .case_insensitive(true)
            .build()
            .ok();
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "\\s", regex.as_ref()), Some(5));
        raw_lines = a.lines_raw(c.pos()..a.len());
        let regex = RegexBuilder::new("\\sLéopard\n.*")
            .size_limit(REGEX_SIZE_LIMIT)
            .case_insensitive(true)
            .build()
            .ok();
        assert_eq!(
            find(&mut c, &mut raw_lines, CaseInsensitive, "\\sLéopard\n.*", regex.as_ref()),
            Some(12)
        );
    }

    #[test]
    fn find_regex_small() {
        let a = Rope::from("Löwe 老虎 Léopard\nSecond line");
        let mut c = Cursor::new(&a, 0);
        let mut raw_lines = a.lines_raw(0..a.len());
        let regex = RegexBuilder::new("L")
            .size_limit(REGEX_SIZE_LIMIT)
            .case_insensitive(false)
            .build()
            .ok();
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "L", regex.as_ref()), Some(0));
        raw_lines = a.lines_raw(c.pos()..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "L", regex.as_ref()), Some(13));
        raw_lines = a.lines_raw(c.pos()..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "L", regex.as_ref()), None);
        c.set(0);
        let regex = RegexBuilder::new("Léopard")
            .size_limit(REGEX_SIZE_LIMIT)
            .case_insensitive(false)
            .build()
            .ok();
        let mut raw_lines = a.lines_raw(0..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "Léopard", regex.as_ref()), Some(13));
        raw_lines = a.lines_raw(c.pos()..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "Léopard", regex.as_ref()), None);
        c.set(0);
        let regex = RegexBuilder::new("老虎")
            .size_limit(REGEX_SIZE_LIMIT)
            .case_insensitive(false)
            .build()
            .ok();
        let mut raw_lines = a.lines_raw(0..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "老虎", regex.as_ref()), Some(6));
        raw_lines = a.lines_raw(c.pos()..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "老虎", regex.as_ref()), None);
        c.set(0);
        let regex = RegexBuilder::new("Tiger")
            .size_limit(REGEX_SIZE_LIMIT)
            .case_insensitive(false)
            .build()
            .ok();
        let mut raw_lines = a.lines_raw(0..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "Tiger", regex.as_ref()), None);
        c.set(0);
        let regex = RegexBuilder::new(".")
            .size_limit(REGEX_SIZE_LIMIT)
            .case_insensitive(false)
            .build()
            .ok();
        let mut raw_lines = a.lines_raw(0..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, Exact, ".", regex.as_ref()), Some(0));
        raw_lines = a.lines_raw(c.pos()..a.len());
        let regex = RegexBuilder::new("\\s")
            .size_limit(REGEX_SIZE_LIMIT)
            .case_insensitive(false)
            .build()
            .ok();
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "\\s", regex.as_ref()), Some(5));
        raw_lines = a.lines_raw(c.pos()..a.len());
        let regex = RegexBuilder::new("\\sLéopard\n.*")
            .size_limit(REGEX_SIZE_LIMIT)
            .case_insensitive(false)
            .build()
            .ok();
        assert_eq!(find(&mut c, &mut raw_lines, Exact, "\\sLéopard\n.*", regex.as_ref()), Some(12));
    }

    #[test]
    fn find_regex_medium() {
        let mut s = String::new();
        for _ in 0..4000 {
            s.push('x');
        }
        s.push_str("Löwe 老虎 Léopard\nSecond line");
        let a = Rope::from(&s);
        let mut c = Cursor::new(&a, 0);
        let mut raw_lines = a.lines_raw(0..a.len());
        let regex =
            RegexBuilder::new("L").size_limit(REGEX_SIZE_LIMIT).case_insensitive(true).build().ok();
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "L", regex.as_ref()), Some(4000));
        raw_lines = a.lines_raw(c.pos()..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "L", regex.as_ref()), Some(4013));
        raw_lines = a.lines_raw(c.pos()..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "L", regex.as_ref()), Some(4029));
        c.set(0);
        let mut raw_lines = a.lines_raw(0..a.len());
        let regex = RegexBuilder::new("Léopard")
            .size_limit(REGEX_SIZE_LIMIT)
            .case_insensitive(true)
            .build()
            .ok();
        assert_eq!(
            find(&mut c, &mut raw_lines, CaseInsensitive, "Léopard", regex.as_ref()),
            Some(4013)
        );
        raw_lines = a.lines_raw(c.pos()..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "Léopard", regex.as_ref()), None);
        c.set(0);
        let regex = RegexBuilder::new("老虎")
            .size_limit(REGEX_SIZE_LIMIT)
            .case_insensitive(true)
            .build()
            .ok();
        let mut raw_lines = a.lines_raw(0..a.len());
        assert_eq!(
            find(&mut c, &mut raw_lines, CaseInsensitive, "老虎", regex.as_ref()),
            Some(4006)
        );
        raw_lines = a.lines_raw(c.pos()..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "老虎", regex.as_ref()), None);
        c.set(0);
        let regex = RegexBuilder::new("Tiger")
            .size_limit(REGEX_SIZE_LIMIT)
            .case_insensitive(true)
            .build()
            .ok();
        let mut raw_lines = a.lines_raw(0..a.len());
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, "Tiger", regex.as_ref()), None);
        c.set(0);
        let mut raw_lines = a.lines_raw(0..a.len());
        let regex =
            RegexBuilder::new(".").size_limit(REGEX_SIZE_LIMIT).case_insensitive(true).build().ok();
        assert_eq!(find(&mut c, &mut raw_lines, CaseInsensitive, ".", regex.as_ref()), Some(0));
        raw_lines = a.lines_raw(c.pos()..a.len());
        let regex = RegexBuilder::new("\\s")
            .size_limit(REGEX_SIZE_LIMIT)
            .case_insensitive(true)
            .build()
            .ok();
        assert_eq!(
            find(&mut c, &mut raw_lines, CaseInsensitive, "\\s", regex.as_ref()),
            Some(4005)
        );
        raw_lines = a.lines_raw(c.pos()..a.len());
        let regex = RegexBuilder::new("\\sLéopard\n.*")
            .size_limit(REGEX_SIZE_LIMIT)
            .case_insensitive(true)
            .build()
            .ok();
        assert_eq!(
            find(&mut c, &mut raw_lines, CaseInsensitive, "\\sLéopard\n.*", regex.as_ref()),
            Some(4012)
        );
    }

    #[test]
    fn compare_cursor_regex_singleline() {
        let regex = Regex::new(r"^(\*+)(?: +(.*?))?[ \t]*$").unwrap();
        let rope = Rope::from("** level 2 headline");
        let mut c = Cursor::new(&rope, 0);
        let mut l = rope.lines_raw(c.pos()..rope.len());
        assert!(compare_cursor_regex(&mut c, &mut l, regex.as_str(), &regex).is_some());

        c.set(3);
        l = rope.lines_raw(c.pos()..rope.len());
        assert!(compare_cursor_regex(&mut c, &mut l, regex.as_str(), &regex).is_none());
    }

    #[test]
    fn compare_cursor_regex_multiline() {
        let regex = Regex::new(
            r"^[ \t]*:PROPERTIES:[ \t]*\n(?:[ \t]*:\S+:(?: .*)?[ \t]*\n)*?[ \t]*:END:[ \t]*\n",
        )
        .unwrap();

        // taken from http://doc.norang.ca/org-mode.html#DiaryForAppointments
        let s = "\
                 #+FILETAGS: PERSONAL\
                 \n* Appointments\
                 \n  :PROPERTIES:\
                 \n  :CATEGORY: Appt\
                 \n  :ARCHIVE:  %s_archive::* Appointments\
                 \n  :END:\
                 \n** Holidays\
                 \n   :PROPERTIES:\
                 \n   :Category: Holiday\
                 \n   :END:\
                 \n   %%(org-calendar-holiday)\
                 \n** Some other Appointment\n";
        let rope = Rope::from(s);
        let mut c = Cursor::new(&rope, 0);
        let mut l = rope.lines_raw(c.pos()..rope.len());
        assert!(compare_cursor_regex(&mut c, &mut l, regex.as_str(), &regex).is_none());

        // move to the next line after "* Appointments"
        c.set(36);
        l = rope.lines_raw(c.pos()..rope.len());
        assert!(compare_cursor_regex(&mut c, &mut l, regex.as_str(), &regex).is_some());
        assert_eq!(117, c.pos());
        assert_eq!(Some('*'), c.next_codepoint());

        // move to the next line after "** Holidays"
        c.set(129);
        l = rope.lines_raw(c.pos()..rope.len());
        assert!(compare_cursor_regex(&mut c, &mut l, regex.as_str(), &regex).is_some());
        c.next_codepoint();
        c.next_codepoint();
        c.next_codepoint();
        assert_eq!(Some('%'), c.next_codepoint());
    }

    #[test]
    fn compare_cursor_str_small() {
        let a = Rope::from("Löwe 老虎 Léopard");
        let mut c = Cursor::new(&a, 0);
        let pat = "Löwe 老虎 Léopard";
        let mut raw_lines = a.lines_raw(0..a.len());
        assert!(compare_cursor_str(&mut c, &mut raw_lines, pat).is_some());
        assert_eq!(c.pos(), pat.len());
        c.set(0);
        let pat = "Löwe";
        assert!(compare_cursor_str(&mut c, &mut raw_lines, pat).is_some());
        assert_eq!(c.pos(), pat.len());
        c.set(0);
        // Empty string is valid for compare_cursor_str (but not find)
        let pat = "";
        assert!(compare_cursor_str(&mut c, &mut raw_lines, pat).is_some());
        assert_eq!(c.pos(), pat.len());
        c.set(0);
        assert!(compare_cursor_str(&mut c, &mut raw_lines, "Löwe 老虎 Léopardfoo").is_none());
    }

    #[test]
    fn compare_cursor_str_medium() {
        let mut s = String::new();
        for _ in 0..4000 {
            s.push('x');
        }
        s.push_str("Löwe 老虎 Léopard");
        let a = Rope::from(&s);
        let mut c = Cursor::new(&a, 0);
        let mut raw_lines = a.lines_raw(0..a.len());
        assert!(compare_cursor_str(&mut c, &mut raw_lines, &s).is_some());
        c.set(2000);
        assert!(compare_cursor_str(&mut c, &mut raw_lines, &s[2000..]).is_some());
    }
}
