// Copyright 2018 The xi-editor Authors.
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

//! Fast comparison of rope regions, principally for diffing.

use rope::{BaseMetric, Rope, RopeInfo};
use tree::{Cursor};


const SSE_STRIDE: usize = 16;

/// Given two 16-byte slices, returns a bitmask where the 1 bits indicate
/// the positions of non-equal bytes.
///
/// The least significant bit in the mask refers to the byte in position 0;
/// that is, you read the mask right to left.
///
/// # Examples
///
/// ```
/// # use xi_rope::compare::fast_cmpestr_mask;
/// # if is_x86_feature_detected!("sse4.2") {
/// let one = "aaaaaaaaaaaaaaaa";
/// let two = "aa3aaaaa9aaaEaaa";
/// let exp = "0001000100000100";
/// let mask = unsafe { fast_cmpestr_mask(one.as_bytes(), two.as_bytes()) };
/// let result = format!("{:016b}", mask);
/// assert_eq!(result.as_str(), exp);
/// # }
/// ```
///
#[inline(always)]
#[doc(hidden)]
#[cfg(target_arch = "x86_64")]
pub unsafe fn fast_cmpestr_mask(one: &[u8], two: &[u8]) -> i32 {
    use std::arch::x86_64::*;

    // https://software.intel.com/sites/landingpage/IntrinsicsGuide/#text=_mm_cmpestrm
    const SSID_OPTS: i32 = _SIDD_CMP_EQUAL_EACH | _SIDD_NEGATIVE_POLARITY;

    debug_assert!(is_x86_feature_detected!("sse4.2"));
    debug_assert_eq!(one.len(), SSE_STRIDE);
    debug_assert_eq!(two.len(), SSE_STRIDE);

    let onev = _mm_loadu_si128(one.as_ptr() as *const _);
    let twov = _mm_loadu_si128(two.as_ptr() as *const _);
    let mask = _mm_cmpestrm(onev, SSE_STRIDE as i32, twov, SSE_STRIDE as i32, SSID_OPTS);
    _mm_cvtsi128_si32(mask)
}

/// Returns the lowest `i` for which `one[i] != two[i]`, if one exists.
pub fn ne_idx(one: &[u8], two: &[u8]) -> Option<usize> {
    if is_x86_feature_detected!("sse4.2") {
        ne_idx_simd(one, two)
    } else {
        ne_idx_fallback(one, two)
    }
}

/// Returns the lowest `i` such that `one[one.len()-i] != two[two.len()-i]`,
/// if one exists.
pub fn ne_idx_rev(one: &[u8], two: &[u8]) -> Option<usize> {
    if is_x86_feature_detected!("sse4.2") {
        ne_idx_rev_simd(one, two)
    } else {
        ne_idx_rev_fallback(one, two)
    }
}

#[cfg(target_arch = "x86_64")]
#[allow(dead_code)]
#[doc(hidden)]
pub fn ne_idx_simd(one: &[u8], two: &[u8]) -> Option<usize> {
    let min_len = one.len().min(two.len());
    let mut idx = 0;
    loop {
        let mask: i32;
        // if slice is less than 16 bytes we manually pad it
        if idx + SSE_STRIDE >= min_len {
            let mut one_buf: [u8; SSE_STRIDE] = [0; SSE_STRIDE];
            let mut two_buf: [u8; SSE_STRIDE] = [0; SSE_STRIDE];
            one_buf[..min_len-idx].copy_from_slice(&one[idx..min_len]);
            two_buf[..min_len-idx].copy_from_slice(&two[idx..min_len]);
            mask = unsafe { fast_cmpestr_mask(&one_buf, &two_buf) };
        } else {
            mask = unsafe { fast_cmpestr_mask(&one[idx..idx+SSE_STRIDE],
                                              &two[idx..idx+SSE_STRIDE]) };
        }
        let i = mask.trailing_zeros() as usize;
        if i != 32 { return Some(idx + i); }
        idx += SSE_STRIDE;
        if idx >= min_len { break; }
    }
    None
}

#[cfg(target_arch = "x86_64")]
#[allow(dead_code)]
#[doc(hidden)]
pub fn ne_idx_rev_simd(one: &[u8], two: &[u8]) -> Option<usize> {
    let min_len = one.len().min(two.len());
    let one = &one[one.len() - min_len..];
    let two = &two[two.len() - min_len..];
    debug_assert_eq!(one.len(), two.len());
    let mut idx = min_len;
    loop {
        let mask: i32;
        if idx < SSE_STRIDE {
            let mut one_buf: [u8; SSE_STRIDE] = [0; SSE_STRIDE];
            let mut two_buf: [u8; SSE_STRIDE] = [0; SSE_STRIDE];
            one_buf[SSE_STRIDE - idx..].copy_from_slice(&one[..idx]);
            two_buf[SSE_STRIDE - idx..].copy_from_slice(&two[..idx]);
            mask = unsafe { fast_cmpestr_mask(&one_buf, &two_buf) };
        } else {
            mask = unsafe { fast_cmpestr_mask(&one[idx-SSE_STRIDE..idx],
                                              &two[idx-SSE_STRIDE..idx]) };
        }
        let i = mask.leading_zeros() as usize - SSE_STRIDE;
        if i != SSE_STRIDE { return Some(min_len - (idx - i)); }
        if idx < SSE_STRIDE { break; }
        idx -= SSE_STRIDE;
    }
    None
}

#[inline]
#[allow(dead_code)]
#[doc(hidden)]
pub fn ne_idx_fallback(one: &[u8], two: &[u8]) -> Option<usize> {
    for i in 0..one.len().min(two.len()) {
        if one[i] != two[i] { return Some(i); }
    }
    None
}

#[inline]
#[allow(dead_code)]
#[doc(hidden)]
pub fn ne_idx_rev_fallback(one: &[u8], two: &[u8]) -> Option<usize> {
    let min_len =  one.len().min(two.len());
    for i in 1..min_len + 1 {
        if one[one.len()-i] != two[two.len()-i] { return Some(i - 1); }
    }
    None
}

/// Utility for efficiently comparing two ropes.
pub struct RopeScanner<'a> {
    base: Cursor<'a, RopeInfo>,
    target: Cursor<'a, RopeInfo>,
    base_chunk: &'a str,
    target_chunk: &'a str,
    scanned: usize,
}

impl<'a> RopeScanner<'a> {
    pub fn new(base: &'a Rope, target: &'a Rope) -> Self {
        RopeScanner {
            base: Cursor::new(base, 0),
            target: Cursor::new(target, 0),
            base_chunk: "",
            target_chunk: "",
            scanned: 0,
        }
    }

    /// Returns the distance to the left of the two offsets to the first codepoint
    /// which is not equal in the two ropes. If no such position exists,
    /// returns the distance to the nearest starting offset.
    ///
    /// if `stop` is not None, the scan will stop at if it reaches this value.
    ///
    /// # Examples
    ///
    /// ```
    /// # use xi_rope::compare::RopeScanner;
    /// # use xi_rope::Rope;
    ///
    /// let one = Rope::from("hiii");
    /// let two = Rope::from("siii");
    /// let mut scanner = RopeScanner::new(&one, &two);
    /// assert_eq!(scanner.find_ne_char_left(one.len(), two.len(), None), 3);
    /// assert_eq!(scanner.find_ne_char_left(one.len(), two.len(), 2), 2);
    /// ```
    pub fn find_ne_char_left<T>(&mut self, base_off: usize, targ_off: usize, stop: T) -> usize
        where T: Into<Option<usize>>
    {
        let stop = stop.into().unwrap_or(usize::max_value());
        self.base.set(base_off);
        self.target.set(targ_off);
        self.scanned = 0;

        let (base_leaf, base_leaf_off) = self.base.get_leaf().unwrap();
        let (target_leaf, target_leaf_off) = self.target.get_leaf().unwrap();

        debug_assert!(self.target.is_boundary::<BaseMetric>());
        debug_assert!(self.base.is_boundary::<BaseMetric>());
        debug_assert!(base_leaf.is_char_boundary(base_leaf_off));
        debug_assert!(target_leaf.is_char_boundary(target_leaf_off));

        self.base_chunk = &base_leaf[..base_leaf_off];
        self.target_chunk = &target_leaf[..target_leaf_off];

        loop {
            if let Some(mut idx) = ne_idx_rev(self.base_chunk.as_bytes(),
                    self.target_chunk.as_bytes()) {
                // find nearest codepoint boundary
                while idx > 1 && !self.base_chunk.is_char_boundary(self.base_chunk.len() - idx) {
                    idx -= 1;
                }
                return stop.min(self.scanned + idx);
            }
            let scan_len = self.target_chunk.len().min(self.base_chunk.len());
            self.base_chunk = &self.base_chunk[..self.base_chunk.len()-scan_len];
            self.target_chunk = &self.target_chunk[..self.target_chunk.len()-scan_len];
            self.scanned += scan_len;

            if stop <= self.scanned { break; }
            self.load_prev_chunk();
            if self.base_chunk.is_empty() || self.target_chunk.is_empty() {
                break;
            }
        }
        stop.min(self.scanned)
    }

    /// Returns the distance to the right of the two offsets to the first
    /// non-equal codepoint. If no such position exists,
    /// returns the distance to the end of the nearest rope.
    ///
    /// if `stop` is not None, the scan will stop at if it reaches this value.
    ///
    /// # Examples
    ///
    /// ```
    /// # use xi_rope::compare::RopeScanner;
    /// # use xi_rope::Rope;
    ///
    /// let one = Rope::from("uh-oh🙈");
    /// let two = Rope::from("uh-oh🙉");
    /// let mut scanner = RopeScanner::new(&one, &two);
    /// assert_eq!(scanner.find_ne_char_right(0, 0, None), 5);
    /// assert_eq!(scanner.find_ne_char_right(0, 0, 3), 3);
    /// ```
    pub fn find_ne_char_right<T>(&mut self, base_off: usize, targ_off: usize, stop: T) -> usize
        where T: Into<Option<usize>>
    {
        let stop = stop.into().unwrap_or(usize::max_value());
        self.base.set(base_off);
        self.target.set(targ_off);
        self.scanned = 0;

        let (base_leaf, base_leaf_off) = self.base.get_leaf().unwrap();
        let (target_leaf, target_leaf_off) = self.target.get_leaf().unwrap();

        debug_assert!(base_leaf.is_char_boundary(base_leaf_off));
        debug_assert!(target_leaf.is_char_boundary(target_leaf_off));

        self.base_chunk = &base_leaf[base_leaf_off..];
        self.target_chunk = &target_leaf[target_leaf_off..];

        loop {
            if let Some(mut idx) = ne_idx(self.base_chunk.as_bytes(),
                    self.target_chunk.as_bytes()) {
                while idx > 0 && !self.base_chunk.is_char_boundary(idx) {
                    idx -= 1;
                }
                return stop.min(self.scanned + idx);
            }
            let scan_len = self.target_chunk.len().min(self.base_chunk.len());
            self.base_chunk = &self.base_chunk[scan_len..];
            self.target_chunk = &self.target_chunk[scan_len..];
            debug_assert!(self.base_chunk.is_empty() || self.target_chunk.is_empty());
            self.scanned += scan_len;
            if stop <= self.scanned { break; }
            self.load_next_chunk();
            if self.base_chunk.is_empty() || self.target_chunk.is_empty() {
                break;
            }
        }
        stop.min(self.scanned)
    }

    /// Returns the positive offset from the start of the rope to the first
    /// non-equal byte, and the negative offest from the end of the rope to
    /// the first non-equal byte.
    ///
    /// # Examples
    ///
    /// ```
    /// # use xi_rope::compare::RopeScanner;
    /// # use xi_rope::Rope;
    ///
    /// let one = Rope::from("123xxx12345");
    /// let two = Rope::from("123ZZZ12345");
    /// let mut scanner = RopeScanner::new(&one, &two);
    /// assert_eq!(scanner.find_min_diff_range(), (3, 5));
    /// ```
    pub fn find_min_diff_range(&mut self) -> (usize, usize) {
        let b_end = self.base.total_len();
        let t_end = self.target.total_len();
        let start = self.find_ne_char_right(0, 0, None);

        // scanning from the end of the document, we should stop at whatever
        // offset we reached scanning from the start.
        let unscanned = b_end.min(t_end) - start;
        if unscanned == 0 {
            debug_assert_eq!(b_end, t_end);
            return (start, start);
        }

        let end = self.find_ne_char_left(b_end, t_end, unscanned);
        (start, end)
    }


    fn load_prev_chunk(&mut self) {
        if self.base_chunk.is_empty() {
            if let Some(prev) = self.base.prev_leaf() {
                self.base_chunk = prev.0;
            }
        }

        if self.target_chunk.is_empty() {
            if let Some(prev) = self.target.prev_leaf() {
                self.target_chunk = prev.0;
            }
        }
    }

    fn load_next_chunk(&mut self) {
        if self.base_chunk.is_empty() {
            if let Some(next) = self.base.next_leaf() {
                self.base_chunk = next.0;
            }
        }

        if self.target_chunk.is_empty() {
            if let Some(next) = self.target.next_leaf() {
                self.target_chunk = next.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::iter;

    #[test]
    fn ne_len() {
        // we should only match up to the length of the shortest input
        let one = "aaaaaa";
        let two = "aaaa";
        let tre = "aaba";
        let fur = "";
        assert!(ne_idx_fallback(one.as_bytes(), two.as_bytes()).is_none());
        assert_eq!(ne_idx_fallback(one.as_bytes(), tre.as_bytes()), Some(2));
        assert_eq!(ne_idx_fallback(one.as_bytes(), fur.as_bytes()), None);
    }

    #[test]
    #[cfg(all(target_arch = "x86_64", target_feature = "sse4.2"))]
    fn ne_len_simd() {
        // we should only match up to the length of the shortest input
        let one = "aaaaaa";
        let two = "aaaa";
        let tre = "aaba";
        let fur = "";
        assert!(ne_idx_simd(one.as_bytes(), two.as_bytes()).is_none());
        assert_eq!(ne_idx_simd(one.as_bytes(), tre.as_bytes()), Some(2));
        assert_eq!(ne_idx_simd(one.as_bytes(), fur.as_bytes()), None);
    }


    #[test]
    fn ne_len_rev() {
        let one = "aaaaaa";
        let two = "aaaa";
        let tre = "aaba";
        let fur = "";
        assert!(ne_idx_rev_fallback(one.as_bytes(), two.as_bytes()).is_none());
        assert_eq!(ne_idx_rev_fallback(one.as_bytes(), tre.as_bytes()), Some(1));
        assert_eq!(ne_idx_rev_fallback(one.as_bytes(), fur.as_bytes()), None);
    }

    #[test]
    #[cfg(all(target_arch = "x86_64", target_feature = "sse4.2"))]
    fn ne_len_rev_simd() {
        let one = "aaaaaa";
        let two = "aaaa";
        let tre = "aaba";
        let fur = "";
        assert!(ne_idx_rev_simd(one.as_bytes(), two.as_bytes()).is_none());
        assert_eq!(ne_idx_rev_simd(one.as_bytes(), tre.as_bytes()), Some(1));
        assert_eq!(ne_idx_rev_simd(one.as_bytes(), fur.as_bytes()), None);
    }

    #[test]
    fn ne_rev_regression1() {
	let one: &[u8] = &[
	    101, 119, 58, 58, 123, 83, 116, 121, 108, 101, 44,
	    32, 86, 105, 101, 119, 125, 59, 10, 10];

	let two: &[u8] = &[
	    101, 119, 58, 58, 123, 83, 101, 32, 118, 105, 101,
	    119, 58, 58, 86, 105, 101, 119, 59, 10];

	assert_eq!(ne_idx_rev_fallback(one, two), Some(1));
        if is_x86_feature_detected!("sse4.2") {
            assert_eq!(ne_idx_rev_simd(one, two), Some(1));
        }
    }

    fn make_lines(n: usize) -> String {
        let mut s = String::with_capacity(n * 81);
        let line: String = iter::repeat('a').take(79).chain(iter::once('\n')).collect();
        for _ in 0..n {
            s.push_str(&line);
        }
        s
    }

    #[test]
    fn scanner_right_simple() {
        let rope =   Rope::from("aaaaaaaaaaaaaaaa");
        let chunk1 = Rope::from("aaaaaaaaaaaaaaaa");
        let chunk2 = Rope::from("baaaaaaaaaaaaaaa");
        let chunk3 = Rope::from("abaaaaaaaaaaaaaa");
        let chunk4 = Rope::from("aaaaaabaaaaaaaaa");
        {
            let mut scanner = RopeScanner::new(&rope, &chunk1);
            assert_eq!(scanner.find_ne_char_right(0, 0, None), 16);
        }

        {
            let mut scanner = RopeScanner::new(&rope, &chunk2);
            assert_eq!(scanner.find_ne_char_right(0, 0, None), 0);
        }

        {
            let mut scanner = RopeScanner::new(&rope, &chunk3);
            assert_eq!(scanner.find_ne_char_right(0, 0, None), 1);
        }

        {
            let mut scanner = RopeScanner::new(&rope, &chunk4);
            assert_eq!(scanner.find_ne_char_right(0, 0, None), 6);
        }
    }

    #[test]
    fn scanner_left_simple() {
        let rope =   Rope::from("aaaaaaaaaaaaaaaa");
        let chunk1 = Rope::from("aaaaaaaaaaaaaaaa");
        let chunk2 = Rope::from("aaaaaaaaaaaaaaba");
        let chunk3 = Rope::from("aaaaaaaaaaaaaaab");
        let chunk4 = Rope::from("aaaaaabaaaaaaaaa");
        {
            let mut scanner = RopeScanner::new(&rope, &chunk1);
            assert_eq!(scanner.find_ne_char_left(rope.len(), chunk1.len(), None), 16);
        }

        {
            let mut scanner = RopeScanner::new(&rope, &chunk2);
            assert_eq!(scanner.find_ne_char_left(rope.len(), chunk2.len(), None), 1);
        }

        {
            let mut scanner = RopeScanner::new(&rope, &chunk3);
            assert_eq!(scanner.find_ne_char_left(rope.len(), chunk3.len(), None), 0);
        }

        {
            let mut scanner = RopeScanner::new(&rope, &chunk4);
            assert_eq!(scanner.find_ne_char_left(rope.len(), chunk4.len(), None), 9);
        }
    }

    #[test]
    fn scan_left_ne_lens() {
        let rope =   Rope::from("aaaaaaaaaaaaaaaa");
        let chunk1 = Rope::from("aaaaaaaaaaaaa");
        let chunk2 = Rope::from("aaaaaaaaaaaaab");

        {
            let mut scanner = RopeScanner::new(&rope, &chunk1);
            assert_eq!(scanner.find_ne_char_left(rope.len(), chunk1.len(), None), 13);
        }

        {
            let mut scanner = RopeScanner::new(&rope, &chunk2);
            assert_eq!(scanner.find_ne_char_left(rope.len(), chunk2.len(), None), 0);
        }
    }

    #[test]
    fn find_diff_range() {
        let one = Rope::from("aaaaaaaaa");
        let two = Rope::from("baaaaaaab");
        let mut scanner = RopeScanner::new(&one, &two);
        let (start, end) = scanner.find_min_diff_range();
        assert_eq!((start, end), (0, 0));

        let one = Rope::from("aaaaaaaaa");
        let two = Rope::from("abaaaaaba");
        let mut scanner = RopeScanner::new(&one, &two);
        let (start, end) = scanner.find_min_diff_range();
        assert_eq!((start, end), (1, 1));
    }

    #[test]
    fn scanner_left() {
        let rope = Rope::from(make_lines(10));
        let mut chunk = String::from("bbb");
        chunk.push_str(&make_lines(5));
        let targ = Rope::from(chunk);

        {
            let mut scanner = RopeScanner::new(&rope, &targ);
            let result = scanner.find_ne_char_left(rope.len(), targ.len(), None);
            assert_eq!(result, 400);
        }

        let mut targ = String::from(targ);
        targ.push('x');
        targ.push('\n');
        let targ = Rope::from(&targ);
        let mut scanner = RopeScanner::new(&rope, &targ);
        let result = scanner.find_ne_char_left(rope.len(), targ.len(), None);
        assert_eq!(result, 1);
    }

    #[test]
    fn find_right_utf8() {
        // make sure we don't include the matching non-boundary bytes
        let one = Rope::from("aaaa🙈");
        let two = Rope::from("aaaa🙉");

        let mut scanner = RopeScanner::new(&one, &two);
        let result = scanner.find_ne_char_right(0, 0, None);
        assert_eq!(result, 4);
    }

    #[test]
    fn find_left_utf8() {
        let zer = Rope::from("baaaa");
        let one = Rope::from("🍄aaaa"); // F0 9F 8D 84 61 61 61 61;
        let two = Rope::from("🙄aaaa"); // F0 9F 99 84 61 61 61 61;
        let tri = Rope::from("🝄aaaa");  // F0 AF 8D 84 61 61 61 61;

        let mut scanner = RopeScanner::new(&zer, &one);
        let result = scanner.find_ne_char_left(zer.len(), one.len(), None);
        assert_eq!(result, 4);

        let mut scanner = RopeScanner::new(&one, &two);
        let result = scanner.find_ne_char_left(one.len(), two.len(), None);
        assert_eq!(result, 4);

        let mut scanner = RopeScanner::new(&one, &tri);
        let result = scanner.find_ne_char_left(one.len(), tri.len(), None);
        assert_eq!(result, 4);
    }

    #[test]
    fn ne_idx_rev_utf8() {
        // there was a weird failure in `find_left_utf8` non_simd, drilling down:
        let zer = "baaaa";
        let one = "🍄aaaa"; // F0 9F 8D 84 61 61 61 61;
        let two = "🙄aaaa"; // F0 9F 99 84 61 61 61 61;
        if is_x86_feature_detected!("sse4.2") {
            assert_eq!(ne_idx_rev_simd(zer.as_bytes(), one.as_bytes()), Some(4));
            assert_eq!(ne_idx_rev_simd(one.as_bytes(), two.as_bytes()), Some(5));
        }
        assert_eq!(ne_idx_rev_fallback(zer.as_bytes(), one.as_bytes()), Some(4));
        assert_eq!(ne_idx_rev_fallback(one.as_bytes(), two.as_bytes()), Some(5));
    }
}
