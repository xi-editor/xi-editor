// Copyright 2018 Google Inc. All rights reserved.
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

//! Module for searching text.

use std::cmp::{min,max};

use serde_json::Value;

use index_set::IndexSet;
use xi_rope::delta::{Delta, DeltaRegion};
use xi_rope::find::{find, CaseMatching};
use xi_rope::rope::{Rope, LinesMetric, RopeInfo};
use xi_rope::tree::Cursor;
use xi_rope::interval::Interval;
use selection::{Selection, SelRegion};
use xi_rope::tree::Metric;

/// Contains logic to search text
pub struct Find {
    // todo: link to search query so that search results can be correlated back to query

    /// The occurrences, which determine the highlights, have been updated.
    hls_dirty: bool,
    /// The currently active search string
    search_string: Option<String>,
    /// The case matching setting for the currently active search
    case_matching: CaseMatching,
    /// The search query should be considered as regular expression
    is_regex: bool,
    /// The set of all known find occurrences (highlights)
    occurrences: Selection,
    /// Set of ranges that have already been searched for the currently active search string
    valid_search: IndexSet,
}

impl Find {
    pub fn new() -> Find {
        Find {
            hls_dirty: true,
            search_string: None,
            case_matching: CaseMatching::CaseInsensitive,
            is_regex: false,
            occurrences: Selection::new(),
            valid_search: IndexSet::new(),
        }
    }

    pub fn occurrences(&self) -> &Selection {
        &self.occurrences
    }

    pub fn hls_dirty(&self) -> bool {
        self.hls_dirty
    }

    pub fn set_hls_dirty(&mut self, is_dirty: bool) {
        self.hls_dirty = is_dirty
    }

    pub fn update_highlights(&mut self, text: &Rope, delta: &Delta<RopeInfo>) {
        // update search highlights for changed regions
        if self.search_string.is_some() {
            self.valid_search = self.valid_search.apply_delta(delta);

            // invalidate occurrences around deletion positions
            for DeltaRegion{ old_offset, new_offset, len } in delta.iter_deletions() {
                self.valid_search.delete_range(new_offset, new_offset + len);
                self.occurrences.delete_range(old_offset, old_offset + len, false);
            }

            self.occurrences = self.occurrences.apply_delta(delta, false, false);

            // invalidate occurrences around insert positions
            for DeltaRegion{ new_offset, len, .. } in delta.iter_inserts() {
                self.valid_search.delete_range(new_offset, new_offset + len);
                self.occurrences.delete_range(new_offset, new_offset + len, false);
            }

            // update find for the whole delta (is going to only update invalid regions)
            let (iv, _) = delta.summary();
            self.update_find(text, iv.start(), iv.end(), true);
        }
    }

    /// Set search parameters and executes the search.
    pub fn do_find(&mut self, text: &Rope, search_string: Option<String>,
                   case_sensitive: bool, is_regex: bool) -> Value {
        if search_string.is_none() {
            self.unset();
            return Value::Null;
        }

        let search_string = search_string.unwrap();
        if search_string.len() == 0 {
            self.unset();
            return Value::Null;
        }

        self.set_find(&search_string, case_sensitive, is_regex);
        self.update_find(text, 0, text.len(), false);

        Value::String(search_string.to_string())
    }

    /// Unsets the search and removes all highlights from the view.
    pub fn unset(&mut self) {
        self.search_string = None;
        self.occurrences = Selection::new();
        self.hls_dirty = true;
        self.valid_search.clear();
    }

    /// Sets find parameters and search query.
    fn set_find(&mut self, search_string: &str, case_sensitive: bool, is_regex: bool) {
        let case_matching = if case_sensitive {
            CaseMatching::Exact
        } else {
            CaseMatching::CaseInsensitive
        };


        if let Some(ref s) = self.search_string {
            if s == search_string && case_matching == self.case_matching && self.is_regex == is_regex {
                // search parameters did not change
                return;
            }
        }

        self.unset();

        self.search_string = Some(search_string.to_string());
        self.case_matching = case_matching;
        self.is_regex = is_regex;
    }

    /// Execute the search on the provided text in the range provided by `start` and `end`.
    pub fn update_find(&mut self, text: &Rope, start: usize, end: usize, include_slop: bool)
    {
        if self.search_string.is_none() {
            return;
        }

        let text_len = text.len();
        // extend the search by twice the string length (twice, because case matching may increase
        // the length of an occurrence)
        let slop = if include_slop { self.search_string.as_ref().unwrap().len() * 2 } else { 0 };
        let mut invalidate_from = None;

        for (_start, end) in self.valid_search.minus_one_range(start, end) {
            let search_string = self.search_string.as_ref().unwrap();

            // expand region to be able to find occurrences around the region's edges
            let from = max(0, slop) - slop;
            let to = min(end + slop, text.len());

            // TODO: this interval might cut a unicode codepoint, make sure it is
            // aligned to codepoint boundaries.
            let text = text.subseq(Interval::new_closed_open(0, to));
            let mut cursor = Cursor::new(&text, from);
            let mut raw_lines = text.lines_raw(from, to);

            while let Some(start) = find(&mut cursor, &mut raw_lines, self.case_matching, &search_string, self.is_regex) {
                let end = cursor.pos();

                let region = SelRegion::new(start, end);
                let prev_len = self.occurrences.len();
                let (_, e) = self.occurrences.add_range_distinct(region);
                // in case of ambiguous search results (e.g. search "aba" in "ababa"),
                // the search result closer to the beginning of the file wins
                if e != end {
                    // Skip the search result and keep the occurrence that is closer to
                    // the beginning of the file. Re-align the cursor to the kept
                    // occurrence
                    cursor.set(e);
                    continue;
                }

                // in case current cursor matches search result (for example query a* matches)
                // all cursor positions, then cursor needs to be increased so that search
                // continues at next position. Otherwise, search will result in overflow since
                // search will always repeat at current cursor position.
                if start == end {
                    // determine whether end of text is reached and stop search or increase
                    // cursor manually
                    if end + 1 >= text_len {
                        break;
                    } else {
                        cursor.set(end + 1);
                    }
                }

                // update line iterator so that line starts at current cursor position
                raw_lines = text.lines_raw(cursor.pos(), to);

                // add_range_distinct() above removes ambiguous regions after the added
                // region, if something has been deleted, everything thereafter is
                // invalidated
                if self.occurrences.len() != prev_len + 1 {
                    invalidate_from = Some(end);
                    self.occurrences.delete_range(end, text_len, false);
                    break;
                }
            }
        }

        if let Some(invalidate_from) = invalidate_from {
            self.valid_search.union_one_range(start, invalidate_from);

            // invalidate all search results from the point of the ambiguous search result until ...
            let is_multi_line = LinesMetric::next(self.search_string.as_ref().unwrap(), 0).is_some();
            if is_multi_line {
                // ... the end of the file
                self.valid_search.delete_range(invalidate_from, text_len);
            } else {
                // ... the end of the line
                let mut cursor = Cursor::new(&text, invalidate_from);
                if let Some(end_of_line) = cursor.next::<LinesMetric>() {
                    self.valid_search.delete_range(invalidate_from, end_of_line);
                }
            }

            // continue with the find for the current region
            self.update_find(text, invalidate_from, end, false);
        } else {
            self.valid_search.union_one_range(start, end);
            self.hls_dirty = true;
        }
    }

    /// Return the occurrence closest to the provided selection `sel`. If searched is reversed then
    /// the occurrence closest to the start of the selection is returned. `wrapped` indicates that
    /// if the end of the text is reached the search continues from the start.
    pub fn next_occurrence(&self, text: &Rope, reverse: bool, wrapped: bool, sel: (usize, usize)) -> Option<SelRegion> {
        let (sel_start, sel_end) = sel;

        if self.occurrences.len() == 0 {
            return None;
        }

        if reverse {
            let next_occurrence = match sel_start.checked_sub(1) {
                Some(search_end) => self.occurrences.regions_in_range(0, search_end).last(),
                None => None
            };

            if next_occurrence.is_none() && !wrapped {
                return self.occurrences.regions_in_range(0, text.len()).last().cloned();
            }

            next_occurrence.cloned()
        } else {
            let next_occurrence = self.occurrences.regions_in_range(sel_end + 1, text.len()).first();

            if next_occurrence.is_none() && !wrapped {
                return self.occurrences.regions_in_range(0, text.len()).first().cloned();
            }

            next_occurrence.cloned()
        }
    }
}