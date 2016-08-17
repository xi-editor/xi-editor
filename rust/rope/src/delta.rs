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

//! A data structure for representing editing operations on ropes.
//! It's useful to explicitly represent these operations so they can be
//! shared across multiple subsystems.

use interval::Interval;
use tree::{Node, NodeInfo, TreeBuilder};
use subset::{Subset, SubsetBuilder};
use std::cmp::min;

pub enum DeltaElement<N: NodeInfo> {
    Copy(usize, usize),  // note: for now, we lose open/closed info at interval endpoints
    Insert(Node<N>),
}

pub struct Delta<N: NodeInfo> {
    els: Vec<DeltaElement<N>>,
    base_len: usize,
}

impl<N: NodeInfo> Delta<N> {
    pub fn simple_edit(interval: Interval, rope: Node<N>, base_len: usize) -> Delta<N> {
        let mut result = Vec::new();
        let (start, end) = interval.start_end();
        if start > 0 {
            result.push(DeltaElement::Copy(0, start));
        }
        if rope.len() > 0 {
            result.push(DeltaElement::Insert(rope));
        }
        if end < base_len {
            result.push(DeltaElement::Copy(end, base_len));
        }
        Delta { els: result, base_len: base_len }
    }

    /// Apply the delta to the given rope. May not work well if the length of the rope
    /// is not compatible with the construction of the delta.
    pub fn apply(&self, base: &Node<N>) -> Node<N> {
        debug_assert_eq!(base.len(), self.base_len);
        let mut b = TreeBuilder::new();
        for elem in &self.els {
            match *elem {
                DeltaElement::Copy(beg, end) => {
                    base.push_subseq(&mut b, Interval::new_closed_open(beg, end))
                }
                DeltaElement::Insert(ref n) => b.push(n.clone())
            }
        }
        b.build()
    }

    /// Factor the delta into an insert-only delta and a subset representing deletions.
    /// Applying the insert then the delete yields the same result as the original delta:
    ///
    /// `let (d1, ss) = d.factor();`
    ///
    /// ss2 = ss.transform_expand(&d1.reverse_insert())
    ///
    /// `ss2.apply_to_string(d1.apply_to_string(s)) == d.apply_to_string(s)`
    pub fn factor(self) -> (Delta<N>, Subset) {
        let mut ins = Vec::new();
        let mut sb = SubsetBuilder::new();
        let mut b1 = 0;
        let mut e1 = 0;
        for elem in self.els {
            match elem {
                DeltaElement::Copy(b, e) => {
                    sb.add_deletion(e1, b);
                    e1 = e;
                }
                DeltaElement::Insert(n) => {
                    if e1 > b1 {
                        ins.push(DeltaElement::Copy(b1, e1));
                    }
                    b1 = e1;
                    ins.push(DeltaElement::Insert(n));
                }
            }
        }
        if b1 < self.base_len {
            ins.push(DeltaElement::Copy(b1, self.base_len));
        }
        sb.add_deletion(e1, self.base_len);
        (Delta { els: ins, base_len: self.base_len }, sb.build())
    }

    /// Synthesize a delta from a "union string" and two subsets, one representing
    /// insertions and the other representing deletions. This is basically the inverse
    /// of `factor`.
    pub fn synthesize(s: &Node<N>, ins: &Subset, del: &Subset) -> Delta<N> {
        let base_len = ins.len(s.len());
        let mut els = Vec::new();
        let mut x = 0;
        let mut ins_ranges = ins.range_iter(s.len());
        let mut last_ins = ins_ranges.next();
        for (b, e) in del.range_iter(s.len()) {
            let mut beg = b;
            while beg < e {
                while let Some((ib, ie)) = last_ins {
                    if ie > beg {
                        break;
                    }
                    x += ie - ib;
                    last_ins = ins_ranges.next();
                }
                if last_ins.is_some() && last_ins.unwrap().0 <= beg {
                    let (ib, ie) = last_ins.unwrap();
                    let end = min(e, ie);
                    let xbeg = beg + x - ib;  // "beg - ib + x" better for overflow?
                    let xend = end + x - ib;  // ditto
                    let merged = if let Some(&mut DeltaElement::Copy(_, ref mut le)) = els.last_mut() {
                        if *le == xbeg {
                            *le = xend;
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if !merged {
                        els.push(DeltaElement::Copy(xbeg, xend));
                    }
                    beg = end;
                } else {
                    let mut end = e;
                    if let Some((ib, _)) = last_ins {
                        end = min(end, ib)
                    }
                    // Note: could try to aggregate insertions, but not sure of the win.
                    els.push(DeltaElement::Insert(s.subseq(Interval::new_closed_open(beg, end))));
                    beg = end;
                }
            }
        }
        Delta { els: els, base_len: base_len }
    }

    /// Do a coordinate transformation on an insert-only delta. The `after` parameter
    /// controls whether the insertions in `self` come after those specific in the
    /// coordinate transform.
    //
    // TODO: write accurate equations
    // TODO: can we infer l from the other inputs?
    pub fn transform_expand(&self, xform: &Subset, l: usize, after: bool) -> Delta<N> {
        let mut els = Vec::new();
        let mut x = 0;  // coordinate within self
        let mut y = 0;  // coordinate within xform
        let mut i = 0;  // index into self.els
        let mut b1 = 0;
        let mut xform_ranges = xform.range_iter(l);
        let mut last_xform = xform_ranges.next();
        while y < l || i < self.els.len() {
            let next_iv_beg = if let Some((xb, _)) = last_xform { xb } else { l };
            if after && y < next_iv_beg {
                y = next_iv_beg;
            }
            while i < self.els.len() {
                match self.els[i] {
                    DeltaElement::Insert(ref n) => {
                        if y > b1 {
                            els.push(DeltaElement::Copy(b1, y));
                        }
                        b1 = y;
                        els.push(DeltaElement::Insert(n.clone()));
                        i += 1;
                    }
                    DeltaElement::Copy(_b, e) => {
                        if y >= next_iv_beg {
                            let mut next_y = e + y - x;
                            if let Some((_, xe)) = last_xform {
                                next_y = min(next_y, xe);
                            }
                            x += next_y - y;
                            y = next_y;
                            if x == e {
                                i += 1;
                            }
                            if let Some((_, xe)) = last_xform {
                                if y == xe {
                                    last_xform = xform_ranges.next();
                                }
                            }
                        }
                        break;
                    }
                }
            }
            if !after && y < next_iv_beg {
                y = next_iv_beg;
            }
        }
        if y > b1 {
            els.push(DeltaElement::Copy(b1, y));
        }
        Delta { els: els, base_len: l }
    }

    /// Return a subset that inverts the insert-only delta:
    ///
    /// `d.invert_insert().apply_to_string(d.apply_to_string(s)) == s`
    pub fn invert_insert(&self) -> Subset {
        let mut sb = SubsetBuilder::new();
        let mut x = 0;
        for elem in &self.els {
            match *elem {
                DeltaElement::Copy(b, e) => {
                    x += e - b;
                }
                DeltaElement::Insert(ref n) => {
                    sb.add_deletion(x, x + n.len());
                    x += n.len();
                }
            }
        }
        sb.build()
    }

    /// Produce a summary of the delta. Everything outside the returned interval
    /// is unchanged, and the old contents of the interval are replaced by new
    /// contents of the returned length. Equations:
    ///
    /// `(iv, new_len) = self.summary()`
    ///
    /// `new_s = self.apply(s)`
    ///
    /// `new_s = simple_edit(iv, new_s.subseq(iv.start(), iv.start() + new_len), s.len()).apply(s)`
    pub fn summary(&self) -> (Interval, usize) {
        let mut els = self.els.as_slice();
        let mut iv_start = 0;
        if let Some((&DeltaElement::Copy(0, end), rest)) = els.split_first() {
            iv_start = end;
            els = rest;
        }
        let mut iv_end = self.base_len;
        if let Some((&DeltaElement::Copy(beg, end), init)) = els.split_last() {
            if end == iv_end {
                iv_end = beg;
                els = init;
            }
        }
        (Interval::new_closed_open(iv_start, iv_end), Delta::els_len(els))
    }

    fn els_len(els: &[DeltaElement<N>]) -> usize {
        els.iter().fold(0, |sum, el|
            sum + match *el {
                DeltaElement::Copy(beg, end) => end - beg,
                DeltaElement::Insert(ref n) => n.len()
            }
        )
    }
}

/// A mapping from coordinates in the source sequence to coordinates in the sequence after
/// the delta is applied.

// TODO: this doesn't need the new strings, so it should either be based on a new structure
// like Delta but missing the strings, or perhaps the two subsets it's synthesized from.
pub struct Transformer<'a, N: NodeInfo + 'a> {
    delta: &'a Delta<N>,
}

impl<'a, N: NodeInfo + 'a> Transformer<'a, N> {
    /// Create a new transformer from a delta.
    pub fn new(delta: &'a Delta<N>) -> Self {
        Transformer {
            delta: delta,
        }
    }

    /// Transform a single coordinate. The `after` parameter indicates whether it
    /// it should land before or after an inserted region.

    // TODO: implement a cursor so we're not scanning from the beginning every time.
    pub fn transform(&mut self, ix: usize, after: bool) -> usize {
        if ix == 0 && !after {
            return 0;
        }
        let mut result = 0;
        for el in &self.delta.els {
            match *el {
                DeltaElement::Copy(beg, end) => {
                    if ix <= beg {
                        return result;
                    }
                    if ix < end || (ix == end && !after) {
                        return result + ix - beg;
                    }
                    result += end - beg;
                }
                DeltaElement::Insert(ref n) => {
                    result += n.len();
                }
            }
        }
        return result;
    }

    /// Determine whether a given interval is untouched by the transformation.
    pub fn interval_untouched(&mut self, iv: Interval) -> bool {
        let mut last_was_ins = true;
        for el in &self.delta.els {
            match *el {
                DeltaElement::Copy(beg, end) => {
                    if iv.is_before(end) {
                        if last_was_ins {
                            if iv.is_after(beg) {
                                return true;
                            }
                        } else {
                            if !iv.is_before(beg) {
                                return true;
                            }
                        }
                    } else {
                        return false;
                    }
                    last_was_ins = false;
                }
                _ => {
                    last_was_ins = true;
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use rope::{Rope, RopeInfo};
    use delta::{Delta};
    use interval::Interval;
    use subset::{Subset, SubsetBuilder};

    const TEST_STR: &'static str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

    // TODO: a clean way of avoiding code duplication without making too much public?
    fn mk_subset(substr: &str, s: &str) -> Subset {
        let mut sb = SubsetBuilder::new();
        let mut j = 0;
        for i in 0..s.len() {
            if j < substr.len() && substr.as_bytes()[j] == s.as_bytes()[i] {
                j += 1;
            } else {
                sb.add_deletion(i, i + 1);
            }
        }
        sb.build()
    }

    impl Delta<RopeInfo> {
        fn apply_to_string(&self, s: &str) -> String {
            String::from(self.apply(&Rope::from(s)))
        }
    }

    #[test]
    fn simple() {
        let d = Delta::simple_edit(Interval::new_closed_open(1, 9), Rope::from("era"), 11);
        assert_eq!("herald", d.apply_to_string("hello world"));
    }

    #[test]
    fn factor() {
        let d = Delta::simple_edit(Interval::new_closed_open(1, 9), Rope::from("era"), 11);
        let (d1, ss) = d.factor();
        assert_eq!("heraello world", d1.apply_to_string("hello world"));
        assert_eq!("hld", ss.apply_to_string("hello world"));
    }

    #[test]
    fn synthesize() {
        let d = Delta::simple_edit(Interval::new_closed_open(1, 9), Rope::from("era"), 11);
        let (d1, del) = d.factor();
        let ins = d1.invert_insert();
        let del = del.transform_expand(&ins);
        let union_str = d1.apply_to_string("hello world");
        let new_d = Delta::synthesize(&Rope::from(&union_str), &ins, &del);
        assert_eq!("herald", new_d.apply_to_string("hello world"));
        let inv_d = Delta::synthesize(&Rope::from(&union_str), &del, &ins);
        assert_eq!("hello world", inv_d.apply_to_string("herald"));
    }

    #[test]
    fn invert_insert() {
        let d = Delta::simple_edit(Interval::new_closed_open(1, 9), Rope::from("era"), 11);
        let (d1, _ss) = d.factor();
        assert_eq!("hello world", d1.invert_insert().apply_to_string("heraello world"));
    }

    #[test]
    fn transform_expand() {
        let str1 = "01259DGJKNQTUVWXYcdefghkmopqrstvwxy";
        let s1 = mk_subset(str1, TEST_STR);
        let d = Delta::simple_edit(Interval::new_closed_open(10, 12), Rope::from("+"), str1.len());
        assert_eq!("01259DGJKN+UVWXYcdefghkmopqrstvwxy", d.apply_to_string(str1));
        let (d2, _ss) = d.factor();
        assert_eq!("01259DGJKN+QTUVWXYcdefghkmopqrstvwxy", d2.apply_to_string(str1));
        let d3 = d2.transform_expand(&s1, TEST_STR.len(), false);
        assert_eq!("0123456789ABCDEFGHIJKLMN+OPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz", d3.apply_to_string(TEST_STR));
        let d4 = d2.transform_expand(&s1, TEST_STR.len(), true);
        assert_eq!("0123456789ABCDEFGHIJKLMNOP+QRSTUVWXYZabcdefghijklmnopqrstuvwxyz", d4.apply_to_string(TEST_STR));
    }
}
