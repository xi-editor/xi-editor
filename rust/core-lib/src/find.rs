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

//! Module for searching text.

use std::cmp::{min,max};

use xi_rope::delta::{Delta, DeltaRegion};
use xi_rope::find::{find, is_multiline_regex, CaseMatching};
use xi_rope::rope::{Rope, LinesMetric, RopeInfo};
use xi_rope::tree::Cursor;
use xi_rope::interval::Interval;
use selection::{Selection, SelRegion};
use xi_rope::tree::Metric;
use regex::{RegexBuilder, Regex};
use word_boundaries::WordCursor;

const REGEX_SIZE_LIMIT: usize = 1000000;

/// Information about search queries and number of matches for find
#[derive(Serialize, Deserialize, Debug)]
pub struct FindStatus {
    /// Identifier for the current search query.
    id: usize,

    /// The current search query.
    chars: Option<String>,

    /// Whether the active search is case matching.
    case_sensitive: Option<bool>,

    /// Whether the search query is considered as regular expression.
    is_regex: Option<bool>,

    /// Query only matches whole words.
    whole_words: Option<bool>,

    /// Total number of matches.
    matches: usize
}

/// Contains logic to search text
pub struct Find {
    /// Uniquely identifies this search query.
    id: usize,
    /// The occurrences, which determine the highlights, have been updated.
    hls_dirty: bool,
    /// The currently active search string.
    search_string: Option<String>,
    /// The case matching setting for the currently active search.
    case_matching: CaseMatching,
    /// The search query should be considered as regular expression.
    regex: Option<Regex>,
    /// Query matches only whole words.
    whole_words: bool,
    /// The set of all known find occurrences (highlights).
    occurrences: Selection,
}

impl Find {
    pub fn new(id: usize) -> Find {
        Find {
            id: id,
            hls_dirty: true,
            search_string: None,
            case_matching: CaseMatching::CaseInsensitive,
            regex: None,
            whole_words: false,
            occurrences: Selection::new(),
        }
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn occurrences(&self) -> &Selection {
        &self.occurrences
    }

    pub fn hls_dirty(&self) -> bool {
        self.hls_dirty
    }

    pub fn find_status(&self, matches_only: bool) -> FindStatus {
        if matches_only {
            FindStatus {
                id: self.id,
                chars: None,
                case_sensitive: None,
                is_regex: None,
                whole_words: None,
                matches: self.occurrences.len(),
            }
        } else {
            FindStatus {
                id: self.id,
                chars: self.search_string.clone(),
                case_sensitive: Some(self.case_matching == CaseMatching::Exact),
                is_regex: Some(self.regex.is_some()),
                whole_words: Some(self.whole_words),
                matches: self.occurrences.len(),
            }
        }
    }

    pub fn set_hls_dirty(&mut self, is_dirty: bool) {
        self.hls_dirty = is_dirty
    }

    pub fn update_highlights(&mut self, text: &Rope, delta: &Delta<RopeInfo>) {
        // update search highlights for changed regions
        if self.search_string.is_some() {
            // invalidate occurrences around deletion positions
            for DeltaRegion{ old_offset, new_offset: _, len } in delta.iter_deletions() {
                self.occurrences.delete_range(old_offset, old_offset + len, false);
            }

            self.occurrences = self.occurrences.apply_delta(delta, false, false);

            // invalidate occurrences around insert positions
            for DeltaRegion{ new_offset, len, .. } in delta.iter_inserts() {
                // also invalidate previous occurrence since it might expand after insertion
                // eg. for regex .* every insertion after match will be part of match
                self.occurrences.delete_range(new_offset.checked_sub(1).unwrap_or(0), new_offset + len, false);
            }

            // update find for the whole delta and everything after
            let (iv, _) = delta.summary();

            // get last valid occurrence that was unaffected by the delta
            let start = match self.occurrences.regions_in_range(0, iv.start()).last() {
                Some(reg) => reg.end,
                None => 0
            };

            // invalidate all search results from the point of the last valid search result until ...
            let is_multi_line = LinesMetric::next(self.search_string.as_ref().unwrap(), 0).is_some();
            let is_multi_line_regex = self.regex.is_some() && is_multiline_regex(self.search_string.as_ref().unwrap());

            if is_multi_line || is_multi_line_regex {
                // ... the end of the file
                self.occurrences.delete_range(iv.start(), text.len(), false);
                self.update_find(text, start, text.len(), false);
            } else {
                // ... the end of the line including line break
                let mut cursor = Cursor::new(&text, iv.end());

                let end_of_line = match cursor.next::<LinesMetric>() {
                    Some(end) => end,
                    None if cursor.pos() == text.len() => cursor.pos(),
                    _ => return
                };

                self.occurrences.delete_range(iv.start(), end_of_line, false);
                self.update_find(text, start, end_of_line, false);
            }
        }
    }

    /// Set search parameters and executes the search.
    pub fn do_find(&mut self, text: &Rope, search_string: &str, case_sensitive: bool,
                   is_regex: bool, whole_words: bool) {
        if search_string.len() == 0 {
            self.unset();
        }

        self.set_find(search_string, case_sensitive, is_regex, whole_words);
        self.update_find(text, 0, text.len(), false);
    }

    /// Unsets the search and removes all highlights from the view.
    pub fn unset(&mut self) {
        self.search_string = None;
        self.occurrences = Selection::new();
        self.hls_dirty = true;
    }

    /// Sets find parameters and search query.
    fn set_find(&mut self, search_string: &str, case_sensitive: bool, is_regex: bool, whole_words: bool) {
        let case_matching = if case_sensitive {
            CaseMatching::Exact
        } else {
            CaseMatching::CaseInsensitive
        };

        if let Some(ref s) = self.search_string {
            if s == search_string && case_matching == self.case_matching &&
                self.regex.is_some() == is_regex && self.whole_words == whole_words {
                // search parameters did not change
                return;
            }
        }

        self.unset();

        self.search_string = Some(search_string.to_string());
        self.case_matching = case_matching;
        self.whole_words = whole_words;

        // create regex from untrusted input
        self.regex = match is_regex {
            false => None,
            true => {
                RegexBuilder::new(search_string)
                    .size_limit(REGEX_SIZE_LIMIT)
                    .case_insensitive(case_matching == CaseMatching::CaseInsensitive)
                    .build()
                    .ok()
            }
        };
    }

    /// Execute the search on the provided text in the range provided by `start` and `end`.
    pub fn update_find(&mut self, text: &Rope, start: usize, end: usize, include_slop: bool)
    {
        if self.search_string.is_none() {
            return;
        }

        // extend the search by twice the string length (twice, because case matching may increase
        // the length of an occurrence)
        let slop = if include_slop { self.search_string.as_ref().unwrap().len() * 2 } else { 0 };

        let search_string = self.search_string.as_ref().unwrap();

        // expand region to be able to find occurrences around the region's edges
        let from = max(start, slop) - slop;
        let to = min(end + slop, text.len());

        // TODO: this interval might cut a unicode codepoint, make sure it is
        // aligned to codepoint boundaries.
        let sub_text = text.subseq(Interval::new_closed_open(0, to));
        let mut find_cursor = Cursor::new(&sub_text, from);
        let mut raw_lines = text.lines_raw(from..to);

        while let Some(start) = find(&mut find_cursor, &mut raw_lines, self.case_matching,
                                     &search_string, &self.regex) {
            let end = find_cursor.pos();

            if self.whole_words && !self.is_matching_whole_words(text, start, end) {
                raw_lines = text.lines_raw(find_cursor.pos()..to);
                continue;
            }

            let region = SelRegion::new(start, end);
            let (_, e) = self.occurrences.add_range_distinct(region);
            // in case of ambiguous search results (e.g. search "aba" in "ababa"),
            // the search result closer to the beginning of the file wins
            if e != end {
                // Skip the search result and keep the occurrence that is closer to
                // the beginning of the file. Re-align the cursor to the kept
                // occurrence
                find_cursor.set(e);
                raw_lines = text.lines_raw(find_cursor.pos()..to);
                continue;
            }

            // in case current cursor matches search result (for example query a* matches)
            // all cursor positions, then cursor needs to be increased so that search
            // continues at next position. Otherwise, search will result in overflow since
            // search will always repeat at current cursor position.
            if start == end {
                // determine whether end of text is reached and stop search or increase
                // cursor manually
                if end + 1 >= text.len() {
                    break;
                } else {
                    find_cursor.set(end + 1);
                }
            }

            // update line iterator so that line starts at current cursor position
            raw_lines = text.lines_raw(find_cursor.pos()..to);
        }

        self.hls_dirty = true;
    }

    /// Return the occurrence closest to the provided selection `sel`. If searched is reversed then
    /// the occurrence closest to the start of the selection is returned. `wrapped` indicates that
    /// if the end of the text is reached the search continues from the start.
    pub fn next_occurrence(&self, text: &Rope, reverse: bool, wrapped: bool, sel: &Selection) -> Option<SelRegion> {
        if self.occurrences.len() == 0 {
            return None;
        }

        let (sel_start, sel_end) = match sel.last() {
            Some(last) if last.is_caret() =>
                // if last selection is caret then allow the current position to be part of the occurrence
                (last.min(), last.max()),
            Some(last) if !last.is_caret() =>
                // if the last selection is not a caret then continue searching after the caret
                (last.min(), last.max() + 1),
            _ => (0, 0)
        };

        if reverse {
            let next_occurrence = match sel_start.checked_sub(1) {
                Some(search_end) => self.occurrences.regions_in_range(0, search_end).last(),
                None => None
            };

            if next_occurrence.is_none() && !wrapped {
                // get previous unselected occurrence
                return self.occurrences.regions_in_range(0, text.len()).iter().cloned().filter(|o| {
                    sel.regions_in_range(o.min(), o.max()).is_empty()
                }).collect::<Vec<SelRegion>>().last().cloned();
            }

            next_occurrence.cloned()
        } else {
            let next_occurrence = self.occurrences.regions_in_range(sel_end, text.len()).first();

            if next_occurrence.is_none() && !wrapped {
                // get next unselected occurrence
                return self.occurrences.regions_in_range(0, text.len()).iter().cloned().filter(|o| {
                    sel.regions_in_range(o.min(), o.max()).is_empty()
                }).collect::<Vec<SelRegion>>().first().cloned();
            }

            next_occurrence.cloned()
        }
    }

    /// Checks if the start and end of a match is matching whole words.
    fn is_matching_whole_words(&self, text: &Rope, start: usize, end: usize) -> bool {
        let mut word_end_cursor = WordCursor::new(text, end - 1);
        let mut word_start_cursor = WordCursor::new(text, start + 1);

        if let Some(start_boundary) = word_start_cursor.prev_boundary() {
            if start_boundary != start {
                return false;
            }
        }

        if let Some(end_boundary) = word_end_cursor.next_boundary() {
            if end_boundary != end {
                return false;
            }
        }

        true
    }
}