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

//! A data structure for representing editing operations on ropes.
//! It's useful to explicitly represent these operations so they can be
//! shared across multiple subsystems.

use crate::interval::{Interval, IntervalBounds};
use crate::multiset::{CountMatcher, Subset, SubsetBuilder};
use crate::tree::{Node, NodeInfo, TreeBuilder};
use std::cmp::min;
use std::fmt;
use std::ops::Deref;
use std::slice;

#[derive(Clone)]
pub enum DeltaElement<N: NodeInfo> {
    /// Represents a range of text in the base document. Includes beginning, excludes end.
    Copy(usize, usize), // note: for now, we lose open/closed info at interval endpoints
    Insert(Node<N>),
}

/// Represents changes to a document by describing the new document as a
/// sequence of sections copied from the old document and of new inserted
/// text. Deletions are represented by gaps in the ranges copied from the old
/// document.
///
/// For example, Editing "abcd" into "acde" could be represented as:
/// `[Copy(0,1),Copy(2,4),Insert("e")]`
#[derive(Clone)]
pub struct Delta<N: NodeInfo> {
    pub els: Vec<DeltaElement<N>>,
    pub base_len: usize,
}

/// A struct marking that a Delta contains only insertions. That is, it copies
/// all of the old document in the same order. It has a `Deref` impl so all
/// normal `Delta` methods can also be used on it.
#[derive(Clone)]
pub struct InsertDelta<N: NodeInfo>(Delta<N>);

impl<N: NodeInfo> Delta<N> {
    pub fn simple_edit<T: IntervalBounds>(interval: T, rope: Node<N>, base_len: usize) -> Delta<N> {
        let mut builder = Builder::new(base_len);
        if rope.is_empty() {
            builder.delete(interval);
        } else {
            builder.replace(interval, rope);
        }
        builder.build()
    }

    /// If this delta represents a simple insertion, returns the inserted node.
    pub fn as_simple_insert(&self) -> Option<&Node<N>> {
        let mut iter = self.els.iter();
        let mut el = iter.next();
        let mut i = 0;
        if let Some(&DeltaElement::Copy(beg, end)) = el {
            if beg != 0 {
                return None;
            }
            i = end;
            el = iter.next();
        }
        if let Some(&DeltaElement::Insert(ref n)) = el {
            el = iter.next();
            if el.is_none() {
                if i == self.base_len {
                    return Some(n);
                }
            } else if let Some(&DeltaElement::Copy(beg, end)) = el {
                if i == beg && end == self.base_len && iter.next().is_none() {
                    return Some(n);
                }
            }
        }
        None
    }

    /// Returns `true` if this delta represents a single deletion without
    /// any insertions.
    ///
    /// Note that this is `false` for the trivial delta, as well as for a deletion
    /// from an empty `Rope`.
    pub fn is_simple_delete(&self) -> bool {
        if self.els.is_empty() {
            return self.base_len > 0;
        }
        if let DeltaElement::Copy(beg, end) = self.els[0] {
            if beg == 0 {
                if self.els.len() == 1 {
                    // Deletion at end
                    end < self.base_len
                } else if let DeltaElement::Copy(b1, e1) = self.els[1] {
                    // Deletion in middle
                    self.els.len() == 2 && end < b1 && e1 == self.base_len
                } else {
                    false
                }
            } else {
                // Deletion at beginning
                end == self.base_len && self.els.len() == 1
            }
        } else {
            false
        }
    }

    /// Returns `true` if applying the delta will cause no change.
    pub fn is_identity(&self) -> bool {
        let len = self.els.len();
        // Case 1: Everything from beginning to end is getting copied.
        if len == 1 {
            if let DeltaElement::Copy(beg, end) = self.els[0] {
                return beg == 0 && end == self.base_len;
            }
        }

        // Case 2: The rope is empty and the entire rope is getting deleted.
        len == 0 && self.base_len == 0
    }

    /// Apply the delta to the given rope. May not work well if the length of the rope
    /// is not compatible with the construction of the delta.
    pub fn apply(&self, base: &Node<N>) -> Node<N> {
        debug_assert_eq!(base.len(), self.base_len, "must apply Delta to Node of correct length");
        let mut b = TreeBuilder::new();
        for elem in &self.els {
            match *elem {
                DeltaElement::Copy(beg, end) => b.push_slice(base, Interval::new(beg, end)),
                DeltaElement::Insert(ref n) => b.push(n.clone()),
            }
        }
        b.build()
    }

    /// Factor the delta into an insert-only delta and a subset representing deletions.
    /// Applying the insert then the delete yields the same result as the original delta:
    ///
    /// ```no_run
    /// # use xi_rope::rope::{Rope, RopeInfo};
    /// # use xi_rope::delta::Delta;
    /// # use std::str::FromStr;
    /// fn test_factor(d : &Delta<RopeInfo>, r : &Rope) {
    ///     let (ins, del) = d.clone().factor();
    ///     let del2 = del.transform_expand(&ins.inserted_subset());
    ///     assert_eq!(String::from(del2.delete_from(&ins.apply(r))), String::from(d.apply(r)));
    /// }
    /// ```
    pub fn factor(self) -> (InsertDelta<N>, Subset) {
        let mut ins = Vec::new();
        let mut sb = SubsetBuilder::new();
        let mut b1 = 0;
        let mut e1 = 0;
        for elem in self.els {
            match elem {
                DeltaElement::Copy(b, e) => {
                    sb.add_range(e1, b, 1);
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
        sb.add_range(e1, self.base_len, 1);
        sb.pad_to_len(self.base_len);
        (InsertDelta(Delta { els: ins, base_len: self.base_len }), sb.build())
    }

    /// Synthesize a delta from a "union string" and two subsets: an old set
    /// of deletions and a new set of deletions from the union. The Delta is
    /// from text to text, not union to union; anything in both subsets will
    /// be assumed to be missing from the Delta base and the new text. You can
    /// also think of these as a set of insertions and one of deletions, with
    /// overlap doing nothing. This is basically the inverse of `factor`.
    ///
    /// Since only the deleted portions of the union string are necessary,
    /// instead of requiring a union string the function takes a `tombstones`
    /// rope which contains the deleted portions of the union string. The
    /// `from_dels` subset must be the interleaving of `tombstones` into the
    /// union string.
    ///
    /// ```no_run
    /// # use xi_rope::rope::{Rope, RopeInfo};
    /// # use xi_rope::delta::Delta;
    /// # use std::str::FromStr;
    /// fn test_synthesize(d : &Delta<RopeInfo>, r : &Rope) {
    ///     let (ins_d, del) = d.clone().factor();
    ///     let ins = ins_d.inserted_subset();
    ///     let del2 = del.transform_expand(&ins);
    ///     let r2 = ins_d.apply(&r);
    ///     let tombstones = ins.complement().delete_from(&r2);
    ///     let d2 = Delta::synthesize(&tombstones, &ins, &del);
    ///     assert_eq!(String::from(d2.apply(r)), String::from(d.apply(r)));
    /// }
    /// ```
    // For if last_old.is_some() && last_old.unwrap().0 <= beg {. Clippy complaints
    // about not using if-let, but that'd change the meaning of the conditional.
    #[allow(clippy::unnecessary_unwrap)]
    pub fn synthesize(tombstones: &Node<N>, from_dels: &Subset, to_dels: &Subset) -> Delta<N> {
        let base_len = from_dels.len_after_delete();
        let mut els = Vec::new();
        let mut x = 0;
        let mut old_ranges = from_dels.complement_iter();
        let mut last_old = old_ranges.next();
        let mut m = from_dels.mapper(CountMatcher::NonZero);
        // For each segment of the new text
        for (b, e) in to_dels.complement_iter() {
            // Fill the whole segment
            let mut beg = b;
            while beg < e {
                // Skip over ranges in old text until one overlaps where we want to fill
                while let Some((ib, ie)) = last_old {
                    if ie > beg {
                        break;
                    }
                    x += ie - ib;
                    last_old = old_ranges.next();
                }
                // If we have a range in the old text with the character at beg, then we Copy
                if last_old.is_some() && last_old.unwrap().0 <= beg {
                    let (ib, ie) = last_old.unwrap();
                    let end = min(e, ie);
                    // Try to merge contiguous Copys in the output
                    let xbeg = beg + x - ib; // "beg - ib + x" better for overflow?
                    let xend = end + x - ib; // ditto
                    let merged =
                        if let Some(&mut DeltaElement::Copy(_, ref mut le)) = els.last_mut() {
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
                    // if the character at beg isn't in the old text, then we Insert
                    // Insert up until the next old range we could Copy from, or the end of this segment
                    let mut end = e;
                    if let Some((ib, _)) = last_old {
                        end = min(end, ib)
                    }
                    // Note: could try to aggregate insertions, but not sure of the win.
                    // Use the mapper to insert the corresponding section of the tombstones rope
                    let interval =
                        Interval::new(m.doc_index_to_subset(beg), m.doc_index_to_subset(end));
                    els.push(DeltaElement::Insert(tombstones.subseq(interval)));
                    beg = end;
                }
            }
        }
        Delta { els, base_len }
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
        (Interval::new(iv_start, iv_end), Delta::total_element_len(els))
    }

    /// Returns the length of the new document. In other words, the length of
    /// the transformed string after this Delta is applied.
    ///
    /// `d.apply(r).len() == d.new_document_len()`
    pub fn new_document_len(&self) -> usize {
        Delta::total_element_len(self.els.as_slice())
    }

    fn total_element_len(els: &[DeltaElement<N>]) -> usize {
        els.iter().fold(0, |sum, el| {
            sum + match *el {
                DeltaElement::Copy(beg, end) => end - beg,
                DeltaElement::Insert(ref n) => n.len(),
            }
        })
    }

    /// Returns the sum length of the inserts of the delta.
    pub fn inserts_len(&self) -> usize {
        self.els.iter().fold(0, |sum, el| {
            sum + match *el {
                DeltaElement::Copy(_, _) => 0,
                DeltaElement::Insert(ref s) => s.len(),
            }
        })
    }

    /// Iterates over all the inserts of the delta.
    pub fn iter_inserts(&self) -> InsertsIter<N> {
        InsertsIter { pos: 0, last_end: 0, els_iter: self.els.iter() }
    }

    /// Iterates over all the deletions of the delta.
    pub fn iter_deletions(&self) -> DeletionsIter<N> {
        DeletionsIter { pos: 0, last_end: 0, base_len: self.base_len, els_iter: self.els.iter() }
    }
}

impl<N: NodeInfo> fmt::Debug for Delta<N>
where
    Node<N>: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if f.alternate() {
            for el in &self.els {
                match *el {
                    DeltaElement::Copy(beg, end) => {
                        write!(f, "{}", "-".repeat(end - beg))?;
                    }
                    DeltaElement::Insert(ref node) => {
                        node.fmt(f)?;
                    }
                }
            }
        } else {
            write!(f, "Delta(")?;
            for el in &self.els {
                match *el {
                    DeltaElement::Copy(beg, end) => {
                        write!(f, "[{},{}) ", beg, end)?;
                    }
                    DeltaElement::Insert(ref node) => {
                        write!(f, "<ins:{}> ", node.len())?;
                    }
                }
            }
            write!(f, "base_len: {})", self.base_len)?;
        }
        Ok(())
    }
}

impl<N: NodeInfo> fmt::Debug for InsertDelta<N>
where
    Node<N>: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<N: NodeInfo> InsertDelta<N> {
    #![allow(clippy::many_single_char_names)]
    /// Do a coordinate transformation on an insert-only delta. The `after` parameter
    /// controls whether the insertions in `self` come after those specific in the
    /// coordinate transform.
    //
    // TODO: write accurate equations
    pub fn transform_expand(&self, xform: &Subset, after: bool) -> InsertDelta<N> {
        let cur_els = &self.0.els;
        let mut els = Vec::new();
        let mut x = 0; // coordinate within self
        let mut y = 0; // coordinate within xform
        let mut i = 0; // index into self.els
        let mut b1 = 0;
        let mut xform_ranges = xform.complement_iter();
        let mut last_xform = xform_ranges.next();
        let l = xform.count(CountMatcher::All);
        while y < l || i < cur_els.len() {
            let next_iv_beg = if let Some((xb, _)) = last_xform { xb } else { l };
            if after && y < next_iv_beg {
                y = next_iv_beg;
            }
            while i < cur_els.len() {
                match cur_els[i] {
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
        InsertDelta(Delta { els, base_len: l })
    }

    // TODO: it is plausible this method also works on Deltas with deletes
    /// Shrink a delta through a deletion of some of its copied regions with
    /// the same base. For example, if `self` applies to a union string, and
    /// `xform` is the deletions from that union, the resulting Delta will
    /// apply to the text.
    pub fn transform_shrink(&self, xform: &Subset) -> InsertDelta<N> {
        let mut m = xform.mapper(CountMatcher::Zero);
        let els = self
            .0
            .els
            .iter()
            .map(|elem| match *elem {
                DeltaElement::Copy(b, e) => {
                    DeltaElement::Copy(m.doc_index_to_subset(b), m.doc_index_to_subset(e))
                }
                DeltaElement::Insert(ref n) => DeltaElement::Insert(n.clone()),
            })
            .collect();
        InsertDelta(Delta { els, base_len: xform.len_after_delete() })
    }

    /// Return a Subset containing the inserted ranges.
    ///
    /// `d.inserted_subset().delete_from_string(d.apply_to_string(s)) == s`
    pub fn inserted_subset(&self) -> Subset {
        let mut sb = SubsetBuilder::new();
        for elem in &self.0.els {
            match *elem {
                DeltaElement::Copy(b, e) => {
                    sb.push_segment(e - b, 0);
                }
                DeltaElement::Insert(ref n) => {
                    sb.push_segment(n.len(), 1);
                }
            }
        }
        sb.build()
    }
}

/// An InsertDelta is a certain kind of Delta, and anything that applies to a
/// Delta that may include deletes also applies to one that definitely
/// doesn't. This impl allows implicit use of those methods.
impl<N: NodeInfo> Deref for InsertDelta<N> {
    type Target = Delta<N>;

    fn deref(&self) -> &Delta<N> {
        &self.0
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
        Transformer { delta }
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
        result
    }

    /// Determine whether a given interval is untouched by the transformation.
    pub fn interval_untouched<T: IntervalBounds>(&mut self, iv: T) -> bool {
        let iv = iv.into_interval(self.delta.base_len);
        let mut last_was_ins = true;
        for el in &self.delta.els {
            match *el {
                DeltaElement::Copy(beg, end) => {
                    if iv.is_before(end) {
                        if last_was_ins {
                            if iv.is_after(beg) {
                                return true;
                            }
                        } else if !iv.is_before(beg) {
                            return true;
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

/// A builder for creating new `Delta` objects.
///
/// Note that all edit operations must be sorted; the start point of each
/// interval must be no less than the end point of the previous one.
pub struct Builder<N: NodeInfo> {
    delta: Delta<N>,
    last_offset: usize,
}

impl<N: NodeInfo> Builder<N> {
    /// Creates a new builder, applicable to a base rope of length `base_len`.
    pub fn new(base_len: usize) -> Builder<N> {
        Builder { delta: Delta { els: Vec::new(), base_len }, last_offset: 0 }
    }

    /// Deletes the given interval. Panics if interval is not properly sorted.
    pub fn delete<T: IntervalBounds>(&mut self, interval: T) {
        let interval = interval.into_interval(self.delta.base_len);
        let (start, end) = interval.start_end();
        assert!(start >= self.last_offset, "Delta builder: intervals not properly sorted");
        if start > self.last_offset {
            self.delta.els.push(DeltaElement::Copy(self.last_offset, start));
        }
        self.last_offset = end;
    }

    /// Replaces the given interval with the new rope. Panics if interval
    /// is not properly sorted.
    pub fn replace<T: IntervalBounds>(&mut self, interval: T, rope: Node<N>) {
        self.delete(interval);
        if !rope.is_empty() {
            self.delta.els.push(DeltaElement::Insert(rope));
        }
    }

    /// Determines if delta would be a no-op transformation if built.
    pub fn is_empty(&self) -> bool {
        self.last_offset == 0 && self.delta.els.is_empty()
    }

    /// Builds the `Delta`.
    pub fn build(mut self) -> Delta<N> {
        if self.last_offset < self.delta.base_len {
            self.delta.els.push(DeltaElement::Copy(self.last_offset, self.delta.base_len));
        }
        self.delta
    }
}

pub struct InsertsIter<'a, N: NodeInfo + 'a> {
    pos: usize,
    last_end: usize,
    els_iter: slice::Iter<'a, DeltaElement<N>>,
}

#[derive(Debug, PartialEq)]
pub struct DeltaRegion {
    pub old_offset: usize,
    pub new_offset: usize,
    pub len: usize,
}

impl DeltaRegion {
    fn new(old_offset: usize, new_offset: usize, len: usize) -> Self {
        DeltaRegion { old_offset, new_offset, len }
    }
}

impl<'a, N: NodeInfo> Iterator for InsertsIter<'a, N> {
    type Item = DeltaRegion;

    fn next(&mut self) -> Option<Self::Item> {
        let mut result = None;
        for elem in &mut self.els_iter {
            match *elem {
                DeltaElement::Copy(b, e) => {
                    self.pos += e - b;
                    self.last_end = e;
                }
                DeltaElement::Insert(ref n) => {
                    result = Some(DeltaRegion::new(self.last_end, self.pos, n.len()));
                    self.pos += n.len();
                    self.last_end += n.len();
                    break;
                }
            }
        }
        result
    }
}

pub struct DeletionsIter<'a, N: NodeInfo + 'a> {
    pos: usize,
    last_end: usize,
    base_len: usize,
    els_iter: slice::Iter<'a, DeltaElement<N>>,
}

impl<'a, N: NodeInfo> Iterator for DeletionsIter<'a, N> {
    type Item = DeltaRegion;

    fn next(&mut self) -> Option<Self::Item> {
        let mut result = None;
        for elem in &mut self.els_iter {
            match *elem {
                DeltaElement::Copy(b, e) => {
                    if b > self.last_end {
                        result = Some(DeltaRegion::new(self.last_end, self.pos, b - self.last_end));
                    }
                    self.pos += e - b;
                    self.last_end = e;
                    if result.is_some() {
                        break;
                    }
                }
                DeltaElement::Insert(ref n) => {
                    self.pos += n.len();
                    self.last_end += n.len();
                }
            }
        }
        if result.is_none() && self.last_end < self.base_len {
            result = Some(DeltaRegion::new(self.last_end, self.pos, self.base_len - self.last_end));
            self.last_end = self.base_len;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use crate::delta::{Builder, Delta, DeltaElement, DeltaRegion};
    use crate::interval::Interval;
    use crate::rope::{Rope, RopeInfo};
    use crate::test_helpers::find_deletions;

    const TEST_STR: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

    #[test]
    fn simple() {
        let d = Delta::simple_edit(Interval::new(1, 9), Rope::from("era"), 11);
        assert_eq!("herald", d.apply_to_string("hello world"));
        assert_eq!(6, d.new_document_len());
    }

    #[test]
    fn factor() {
        let d = Delta::simple_edit(Interval::new(1, 9), Rope::from("era"), 11);
        let (d1, ss) = d.factor();
        assert_eq!("heraello world", d1.apply_to_string("hello world"));
        assert_eq!("hld", ss.delete_from_string("hello world"));
    }

    #[test]
    fn synthesize() {
        let d = Delta::simple_edit(Interval::new(1, 9), Rope::from("era"), 11);
        let (d1, del) = d.factor();
        let ins = d1.inserted_subset();
        let del = del.transform_expand(&ins);
        let union_str = d1.apply_to_string("hello world");
        let tombstones = ins.complement().delete_from_string(&union_str);
        let new_d = Delta::synthesize(&Rope::from(&tombstones), &ins, &del);
        assert_eq!("herald", new_d.apply_to_string("hello world"));
        let text = del.complement().delete_from_string(&union_str);
        let inv_d = Delta::synthesize(&Rope::from(&text), &del, &ins);
        assert_eq!("hello world", inv_d.apply_to_string("herald"));
    }

    #[test]
    fn inserted_subset() {
        let d = Delta::simple_edit(Interval::new(1, 9), Rope::from("era"), 11);
        let (d1, _ss) = d.factor();
        assert_eq!("hello world", d1.inserted_subset().delete_from_string("heraello world"));
    }

    #[test]
    fn transform_expand() {
        let str1 = "01259DGJKNQTUVWXYcdefghkmopqrstvwxy";
        let s1 = find_deletions(str1, TEST_STR);
        let d = Delta::simple_edit(Interval::new(10, 12), Rope::from("+"), str1.len());
        assert_eq!("01259DGJKN+UVWXYcdefghkmopqrstvwxy", d.apply_to_string(str1));
        let (d2, _ss) = d.factor();
        assert_eq!("01259DGJKN+QTUVWXYcdefghkmopqrstvwxy", d2.apply_to_string(str1));
        let d3 = d2.transform_expand(&s1, false);
        assert_eq!(
            "0123456789ABCDEFGHIJKLMN+OPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz",
            d3.apply_to_string(TEST_STR)
        );
        let d4 = d2.transform_expand(&s1, true);
        assert_eq!(
            "0123456789ABCDEFGHIJKLMNOP+QRSTUVWXYZabcdefghijklmnopqrstuvwxyz",
            d4.apply_to_string(TEST_STR)
        );
    }

    #[test]
    fn transform_shrink() {
        let d = Delta::simple_edit(Interval::new(10, 12), Rope::from("+"), TEST_STR.len());
        let (d2, _ss) = d.factor();
        assert_eq!(
            "0123456789+ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz",
            d2.apply_to_string(TEST_STR)
        );

        let str1 = "0345678BCxyz";
        let s1 = find_deletions(str1, TEST_STR);
        let d3 = d2.transform_shrink(&s1);
        assert_eq!("0345678+BCxyz", d3.apply_to_string(str1));

        let str2 = "356789ABCx";
        let s2 = find_deletions(str2, TEST_STR);
        let d4 = d2.transform_shrink(&s2);
        assert_eq!("356789+ABCx", d4.apply_to_string(str2));
    }

    #[test]
    fn iter_inserts() {
        let mut builder = Builder::new(10);
        builder.replace(Interval::new(2, 2), Rope::from("a"));
        builder.delete(Interval::new(3, 5));
        builder.replace(Interval::new(6, 8), Rope::from("b"));
        let delta = builder.build();

        assert_eq!("01a25b89", delta.apply_to_string("0123456789"));

        let mut iter = delta.iter_inserts();
        assert_eq!(Some(DeltaRegion::new(2, 2, 1)), iter.next());
        assert_eq!(Some(DeltaRegion::new(6, 5, 1)), iter.next());
        assert_eq!(None, iter.next());
    }

    #[test]
    fn iter_deletions() {
        let mut builder = Builder::new(10);
        builder.delete(Interval::new(0, 2));
        builder.delete(Interval::new(4, 6));
        builder.delete(Interval::new(8, 10));
        let delta = builder.build();

        assert_eq!("2367", delta.apply_to_string("0123456789"));

        let mut iter = delta.iter_deletions();
        assert_eq!(Some(DeltaRegion::new(0, 0, 2)), iter.next());
        assert_eq!(Some(DeltaRegion::new(4, 2, 2)), iter.next());
        assert_eq!(Some(DeltaRegion::new(8, 4, 2)), iter.next());
        assert_eq!(None, iter.next());
    }

    #[test]
    fn fancy_bounds() {
        let mut builder = Builder::new(10);
        builder.delete(..2);
        builder.delete(4..=5);
        builder.delete(8..);
        let delta = builder.build();
        assert_eq!("2367", delta.apply_to_string("0123456789"));
    }

    #[test]
    fn is_simple_delete() {
        let d = Delta::simple_edit(10..12, Rope::from("+"), TEST_STR.len());
        assert_eq!(false, d.is_simple_delete());

        let d = Delta::simple_edit(Interval::new(0, 0), Rope::from(""), 0);
        assert_eq!(false, d.is_simple_delete());

        let d = Delta::simple_edit(Interval::new(10, 11), Rope::from(""), TEST_STR.len());
        assert_eq!(true, d.is_simple_delete());

        let mut builder = Builder::<RopeInfo>::new(10);
        builder.delete(Interval::new(0, 2));
        builder.delete(Interval::new(4, 6));
        let d = builder.build();
        assert_eq!(false, d.is_simple_delete());

        let builder = Builder::<RopeInfo>::new(10);
        let d = builder.build();
        assert_eq!(false, d.is_simple_delete());

        let delta = Delta {
            els: vec![
                DeltaElement::Copy(0, 10),
                DeltaElement::Copy(12, 20),
                DeltaElement::Insert(Rope::from("hi")),
            ],
            base_len: 20,
        };

        assert!(!delta.is_simple_delete());
    }

    #[test]
    fn is_identity() {
        let d = Delta::simple_edit(10..12, Rope::from("+"), TEST_STR.len());
        assert_eq!(false, d.is_identity());

        let d = Delta::simple_edit(0..0, Rope::from(""), TEST_STR.len());
        assert_eq!(true, d.is_identity());

        let d = Delta::simple_edit(0..0, Rope::from(""), 0);
        assert_eq!(true, d.is_identity());
    }

    #[test]
    fn as_simple_insert() {
        let d = Delta::simple_edit(Interval::new(10, 11), Rope::from("+"), TEST_STR.len());
        assert_eq!(None, d.as_simple_insert());

        let d = Delta::simple_edit(Interval::new(10, 10), Rope::from("+"), TEST_STR.len());
        assert_eq!(Some(Rope::from("+")).as_ref(), d.as_simple_insert());
    }
}

#[cfg(all(test, feature = "serde"))]
mod serde_tests {
    use crate::rope::{Rope, RopeInfo};
    use crate::{Delta, Interval};
    use serde_json;

    const TEST_STR: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

    #[test]
    fn delta_serde() {
        let d = Delta::simple_edit(Interval::new(10, 12), Rope::from("+"), TEST_STR.len());
        let ser = serde_json::to_value(d.clone()).expect("serialize failed");
        eprintln!("{:?}", &ser);
        let de: Delta<RopeInfo> = serde_json::from_value(ser).expect("deserialize failed");
        assert_eq!(d.apply_to_string(TEST_STR), de.apply_to_string(TEST_STR));
    }
}
