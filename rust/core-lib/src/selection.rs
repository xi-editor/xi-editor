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

//! Data structures representing (multiple) selections and cursors.

use std::cmp::{min, max};
use std::ops::Deref;
use std::ops::Bound;
use std::ops::RangeBounds;

use index_set::remove_n_at;
use xi_rope::delta::{Delta, Transformer};
use xi_rope::rope::RopeInfo;

/// A type representing horizontal measurements. This is currently in units
/// that are not very well defined except that ASCII characters count as
/// 1 each. It will change.
pub type HorizPos = usize;

/// A set of zero or more selection regions, representing a selection state.
#[derive(Default, Debug, Clone)]
pub struct Selection {
    // An invariant: regions[i].max() <= regions[i+1].min()
    // and < if either is_caret()
    regions: Vec<SelRegion>,
}

impl Selection {
    /// Creates a new empty selection.
    pub fn new() -> Selection {
        Selection::default()
    }

    /// Creates a selection with a single region.
    pub fn new_simple(region: SelRegion) -> Selection {
        Selection {
            regions: vec![region]
        }
    }

    /// Clear the selection.
    pub fn clear(&mut self) {
        self.regions.clear();
    }

    /// Collapse all selections into a single caret.
    pub fn collapse(&mut self) {
        self.regions.truncate(1);
        self.regions[0].start = self.regions[0].end;
    }

    // The smallest index so that offset > region.max() for all preceding
    // regions.
    pub fn search(&self, offset: usize) -> usize {
        if self.regions.is_empty() || offset > self.regions.last().unwrap().max() {
            return self.regions.len();
        }
        match self.regions.binary_search_by(|r| r.max().cmp(&offset)) {
            Ok(ix) => ix,
            Err(ix) => ix,
        }
    }

    /// Add a region to the selection. This method implements merging logic.
    ///
    /// Two non-caret regions merge if their interiors intersect; merely
    /// touching at the edges does not cause a merge. A caret merges with
    /// a non-caret if it is in the interior or on either edge. Two carets
    /// merge if they are the same offset.
    ///
    /// Performance note: should be O(1) if the new region strictly comes
    /// after all the others in the selection, otherwise O(n).
    pub fn add_region(&mut self, region: SelRegion) {
        let mut ix = self.search(region.min());
        if ix == self.regions.len() {
            self.regions.push(region);
            return;
        }
        let mut region = region;
        let mut end_ix = ix;
        if self.regions[ix].min() <= region.min() {
            if self.regions[ix].should_merge(region) {
                region = self.regions[ix].merge_with(region);
            } else {
                ix += 1;
            }
            end_ix += 1;
        }
        while end_ix < self.regions.len() && region.should_merge(self.regions[end_ix]) {
            region = region.merge_with(self.regions[end_ix]);
            end_ix += 1;
        }
        if ix == end_ix {
            self.regions.insert(ix, region);
        } else {
            self.regions[ix] = region;
            remove_n_at(&mut self.regions, ix + 1, end_ix - ix - 1);
        }
    }

    /// Gets a slice of regions that intersect the given range. Regions that
    /// merely touch the range at the edges are also included, so it is the
    /// caller's responsibility to further trim them, in particular to only
    /// display one caret in the upstream/downstream cases.
    ///
    /// Performance note: O(log n).
    pub fn regions_in_range(&self, start: usize, end: usize) -> &[SelRegion] {
        let first = self.search(start);
        let mut last = self.search(end);
        if last < self.regions.len() && self.regions[last].min() <= end {
            last += 1;
        }
        &self.regions[first..last]
    }

    /// Deletes all the regions that intersect or (if delete_adjacent = true) touch the given range.
    pub fn delete_range(&mut self, start: usize, end: usize, delete_adjacent: bool) {
        let mut first = self.search(start);
        let mut last = self.search(end);
        if first >= self.regions.len() {
            return;
        }
        if !delete_adjacent && self.regions[first].max() == start {
            first += 1;
        }
        if last < self.regions.len() && ((delete_adjacent && self.regions[last].min() <= end)
           || (!delete_adjacent && self.regions[last].min() < end)) {
            last += 1;
        }
        remove_n_at(&mut self.regions, first, last - first);
    }

    /// Add a region to the selection. This method does not merge regions and does not allow
    /// ambiguous regions (regions that overlap).
    ///
    /// On ambiguous regions, the region with the lower start position wins. That is, in such a
    /// case, the new region is either not added at all, because there is an ambiguous region with
    /// a lower start position, or existing regions that intersect with the new region but do
    /// not start before the new region, are deleted.
    pub fn add_range_distinct(&mut self, region: SelRegion) -> (usize, usize) {
        let mut ix = self.search(region.min());

        if ix < self.regions.len() && self.regions[ix].max() == region.min() {
            ix += 1;
        }

        if ix < self.regions.len() {
            // in case of ambiguous regions the region closer to the left wins
            let occ = &self.regions[ix];
            let is_eq = occ.min() == region.min() && occ.max() == region.max();
            let is_intersect_before = region.min() >= occ.min() && occ.max() > region.min();
            if is_eq || is_intersect_before {
                return (occ.min(), occ.max());
            }
        }

        // delete ambiguous regions to the right
        let mut last = self.search(region.max());
        if last < self.regions.len() && self.regions[last].min() < region.max() {
            last += 1;
        }
        remove_n_at(&mut self.regions, ix, last - ix);

        if ix == self.regions.len() {
            self.regions.push(region);
        } else {
            self.regions.insert(ix, region);
        }

        (self.regions[ix].min(), self.regions[ix].max())
    }

    /// Computes a new selection based on applying a delta to the old selection.
    ///
    /// When new text is inserted at a caret, the new caret can be either before
    /// or after the inserted text, depending on the `after` parameter.
    ///
    /// Whether or not the preceding selections are restored depends on the keep_selections
    /// value (only set to true on transpose).
    pub fn apply_delta(&self, delta: &Delta<RopeInfo>, after: bool, keep_selections: bool) -> Selection {
        let mut result = Selection::new();
        let mut transformer = Transformer::new(delta);
        for region in self.iter() {
            let preserve_selection = keep_selections && region.start != region.end;
            let start_after = {
                if preserve_selection {
                    region.start > region.end
                } else {
                    after
                }
            };
            let end_after = {
                if preserve_selection {
                    region.start < region.end
                } else {
                    after
                }
            };

            let new_region = SelRegion::new(
                transformer.transform(region.start, start_after),
                transformer.transform(region.end, end_after),
            ).with_affinity(region.affinity);
            result.add_region(new_region);
        }
        result
    }

}

/// Implementing the Deref trait allows callers to easily test `is_empty`, iterate
/// through all ranges, etc.
impl Deref for Selection {
    type Target = [SelRegion];

    fn deref(&self) -> &[SelRegion] {
        &self.regions
    }
}

/// The "affinity" of a cursor which is sitting exactly on a line break.
///
/// We say "cursor" here rather than "caret" because (depending on presentation)
/// the front-end may draw a cursor even when the region is not a caret.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Affinity {
    /// The cursor should be displayed downstream of the line break. For
    /// example, if the buffer is "abcd", and the cursor is on a line break
    /// after "ab", it should be displayed on the second line before "cd".
    Downstream,
    /// The cursor should be displayed upstream of the line break. For
    /// example, if the buffer is "abcd", and the cursor is on a line break
    /// after "ab", it should be displayed on the previous line after "ab".
    Upstream,
}

impl Default for Affinity {
    fn default() -> Affinity {
        Affinity::Downstream
    }
}

/// A type representing a single contiguous region of a selection. We use the
/// term "caret" (sometimes also "cursor", more loosely) to refer to a selection
/// region with an empty interior. A "non-caret region" is one with a non-empty
/// interior (i.e. `start != end`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SelRegion {
    /// The inactive edge of a selection, as a byte offset. When
    /// equal to end, the selection range acts as a caret.
    pub start: usize,

    /// The active edge of a selection, as a byte offset.
    pub end: usize,

    /// A saved horizontal position (used primarily for line up/down movement).
    pub horiz: Option<HorizPos>,

    /// The affinity of the cursor.
    pub affinity: Affinity,
}

impl SelRegion {
    /// Returns a new region.
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            horiz: None,
            affinity: Affinity::default(),
        }
    }

    /// Returns a new caret region (`start == end`).
    pub fn caret(pos: usize) -> Self {
        Self {
            start: pos,
            end: pos,
            horiz: None,
            affinity: Affinity::default(),
        }
    }

    /// Returns a region with the given horizontal position.
    pub fn with_horiz(self, horiz: Option<HorizPos>) -> Self {
        Self {
            horiz,
            ..self
        }
    }

    /// Returns a region with the given affinity.
    pub fn with_affinity(self, affinity: Affinity) -> Self {
        Self {
            affinity,
            ..self
        }
    }

    /// Gets the earliest offset within the region, ie the minimum of both edges.
    pub fn min(self) -> usize {
        min(self.start, self.end)
    }

    /// Gets the latest offset within the region, ie the maximum of both edges.
    pub fn max(self) -> usize {
        max(self.start, self.end)
    }

    /// Determines whether the region is a caret (ie has an empty interior).
    pub fn is_caret(self) -> bool {
        self.start == self.end
    }

    /// Determines whether the region's affinity is upstream.
    pub fn is_upstream(self) -> bool {
        self.affinity == Affinity::Upstream
    }

    // Indicate whether this region should merge with the next.
    // Assumption: regions are sorted (self.min() <= other.min())
    fn should_merge(self, other: SelRegion) -> bool {
        other.min() < self.max() ||
            ((self.is_caret() || other.is_caret()) && other.min() == self.max())
    }

    fn merge_with(self, other: SelRegion) -> SelRegion {
        let is_forward = self.end > self.start || other.end > other.start;
        let new_min = min(self.min(), other.min());
        let new_max = max(self.max(), other.max());
        let (start, end) = if is_forward {
            (new_min, new_max)
        } else {
            (new_max, new_min)
        };
        // Could try to preserve horiz/affinity from one of the
        // sources, but very likely not worth it.
        SelRegion::new(start, end)
    }
}

// Returns `[min..max)`
impl<'a> RangeBounds<usize> for &'a SelRegion {
    fn start_bound(&self) -> Bound<&usize> {
        Bound::Included(min(&self.start, &self.end))
    }
    
    fn end_bound(&self) -> Bound<&usize> {
        Bound::Excluded(max(&self.start, &self.end))
    }
}

impl From<SelRegion> for Selection {
    fn from(region: SelRegion) -> Self {
        Self::new_simple(region)
    }
}

#[cfg(test)]
mod tests {
    use super::{Selection, SelRegion};
    use std::ops::Deref;

    fn r(start: usize, end: usize) -> SelRegion {
        SelRegion::new(start, end)
    }

    #[test]
    fn empty() {
        let s = Selection::new();
        assert!(s.is_empty());
        assert_eq!(s.deref(), &[]);
    }

    #[test]
    fn simple_region() {
        let s = Selection::new_simple(r(3, 5));
        assert!(!s.is_empty());
        assert_eq!(s.deref(), &[r(3, 5)]);
    }

    #[test]
    fn from_selregion() {
        let s: Selection = r(3, 5).into();
        assert!(!s.is_empty());
        assert_eq!(s.deref(), &[r(3, 5)]);
    }

    #[test]
    fn delete_range() {
        let mut s = Selection::new_simple(r(3, 5));
        s.delete_range(1, 2, true);
        assert_eq!(s.deref(), &[r(3, 5)]);
        s.delete_range(1, 3, false);
        assert_eq!(s.deref(), &[r(3, 5)]);
        s.delete_range(1, 3, true);
        assert_eq!(s.deref(), &[]);

        let mut s = Selection::new_simple(r(3, 5));
        s.delete_range(5, 6, false);
        assert_eq!(s.deref(), &[r(3, 5)]);
        s.delete_range(5, 6, true);
        assert_eq!(s.deref(), &[]);

        let mut s = Selection::new_simple(r(3, 5));
        s.delete_range(2, 4, false);
        assert_eq!(s.deref(), &[]);
        assert_eq!(s.deref(), &[]);

        let mut s = Selection::new();
        s.add_region(r(3, 5));
        s.add_region(r(7, 8));
        s.delete_range(2, 10, false);
        assert_eq!(s.deref(), &[]);
    }

    #[test]
    fn simple_regions_in_range() {
        let s = Selection::new_simple(r(3, 5));
        assert_eq!(s.regions_in_range(0, 1), &[]);
        assert_eq!(s.regions_in_range(0, 2), &[]);
        assert_eq!(s.regions_in_range(0, 3), &[r(3, 5)]);
        assert_eq!(s.regions_in_range(0, 4), &[r(3, 5)]);
        assert_eq!(s.regions_in_range(5, 6), &[r(3, 5)]);
        assert_eq!(s.regions_in_range(6, 7), &[]);
    }

    #[test]
    fn caret_regions_in_range() {
        let s = Selection::new_simple(r(4, 4));
        assert_eq!(s.regions_in_range(0, 1), &[]);
        assert_eq!(s.regions_in_range(0, 2), &[]);
        assert_eq!(s.regions_in_range(0, 3), &[]);
        assert_eq!(s.regions_in_range(0, 4), &[r(4, 4)]);
        assert_eq!(s.regions_in_range(4, 4), &[r(4, 4)]);
        assert_eq!(s.regions_in_range(4, 5), &[r(4, 4)]);
        assert_eq!(s.regions_in_range(5, 6), &[]);
    }

    #[test]
    fn merge_regions() {
        let mut s = Selection::new();
        s.add_region(r(3, 5));
        assert_eq!(s.deref(), &[r(3, 5)]);
        s.add_region(r(7, 9));
        assert_eq!(s.deref(), &[r(3, 5), r(7, 9)]);
        s.add_region(r(1, 3));
        assert_eq!(s.deref(), &[r(1, 3), r(3, 5), r(7, 9)]);
        s.add_region(r(4, 6));
        assert_eq!(s.deref(), &[r(1, 3), r(3, 6), r(7, 9)]);
        s.add_region(r(2, 8));
        assert_eq!(s.deref(), &[r(1, 9)]);

        s.clear();
        assert_eq!(s.deref(), &[]);
        s.add_region(r(1, 4));
        s.add_region(r(4, 5));
        s.add_region(r(5, 6));
        s.add_region(r(6, 9));
        assert_eq!(s.deref(), &[r(1, 4), r(4, 5), r(5, 6), r(6, 9)]);
        s.add_region(r(2, 8));
        assert_eq!(s.deref(), &[r(1, 9)]);
    }

    #[test]
    fn merge_carets() {
        let mut s = Selection::new();
        s.add_region(r(1, 1));
        assert_eq!(s.deref(), &[r(1, 1)]);
        s.add_region(r(3, 3));
        assert_eq!(s.deref(), &[r(1, 1), r(3, 3)]);
        s.add_region(r(2, 2));
        assert_eq!(s.deref(), &[r(1, 1), r(2, 2), r(3, 3)]);
        s.add_region(r(1, 1));
        assert_eq!(s.deref(), &[r(1, 1), r(2, 2), r(3, 3)]);
    }

    #[test]
    fn merge_region_caret() {
        let mut s = Selection::new();
        s.add_region(r(3, 5));
        assert_eq!(s.deref(), &[r(3, 5)]);
        s.add_region(r(3, 3));
        assert_eq!(s.deref(), &[r(3, 5)]);
        s.add_region(r(4, 4));
        assert_eq!(s.deref(), &[r(3, 5)]);
        s.add_region(r(5, 5));
        assert_eq!(s.deref(), &[r(3, 5)]);
        s.add_region(r(6, 6));
        assert_eq!(s.deref(), &[r(3, 5), r(6, 6)]);
    }

    #[test]
    fn merge_reverse() {
        let mut s = Selection::new();
        s.add_region(r(5, 3));
        assert_eq!(s.deref(), &[r(5, 3)]);
        s.add_region(r(9, 7));
        assert_eq!(s.deref(), &[r(5, 3), r(9, 7)]);
        s.add_region(r(3, 1));
        assert_eq!(s.deref(), &[r(3, 1), r(5, 3), r(9, 7)]);
        s.add_region(r(6, 4));
        assert_eq!(s.deref(), &[r(3, 1), r(6, 3), r(9, 7)]);
        s.add_region(r(8, 2));
        assert_eq!(s.deref(), &[r(9, 1)]);
    }
}
