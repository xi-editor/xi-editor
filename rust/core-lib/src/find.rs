use std::cmp::{min,max};
use std::mem;
use std::cell::RefCell;
use std::ops::Range;

use serde_json::Value;

use index_set::IndexSet;
use xi_rope::delta::{Delta, DeltaRegion};
use xi_rope::find::{find, CaseMatching};
use xi_rope::rope::{Rope, LinesMetric, RopeInfo};
use xi_rope::tree::{Cursor, Metric};
use xi_rope::breaks::{Breaks, BreaksInfo, BreaksMetric, BreaksBaseMetric};
use xi_rope::interval::Interval;
use xi_rope::spans::Spans;
use xi_trace::trace_block;
use client::Client;
use edit_types::ViewEvent;
use line_cache_shadow::{self, LineCacheShadow, RenderPlan, RenderTactic};
use movement::{Movement, region_movement, selection_movement};
use rpc::{GestureType, MouseAction};
use styles::{Style, ThemeStyleMap};
use selection::{Affinity, Selection, SelRegion};
use tabs::{ViewId, BufferId};
use width_cache::WidthCache;
use word_boundaries::WordCursor;

pub struct SearchQuery {
  id: usize,   // necessary?
  query: String,
  case_matching: CaseMatching // todo: add regex
}

pub struct SearchOccurrence {
  query_id: usize,      // id or reference to SearchQuery?
  highlight: Selection
}




const BACKWARDS_FIND_CHUNK_SIZE: usize = 32_768;

pub struct Find {
  // todo: support multiple queries
  //  search_queries: Vec<SearchQuery>,

  // todo: occurrences in separate type that references back to the search query (required for coloring, ...)
  //  occurrences: Vec<SearchOccurrence>,

  // todo: the following will be deprecated after adding support for multiple queries
  /// The occurrences, which determine the highlights, have been updated.
  hls_dirty: bool,
  /// The currently active search string
  search_string: Option<String>,
  /// The case matching setting for the currently active search
  case_matching: CaseMatching,
  /// The set of all known find occurrences (highlights)
  occurrences: Option<Selection>,
  /// Set of ranges that have already been searched for the currently active search string
  valid_search: IndexSet,
}



impl Find {
  pub fn new() -> Find {
    Find {
//      search_queries: Vec::new(),
//      occurrences: Vec::new(),
      hls_dirty: true,
      search_string: None,
      case_matching: CaseMatching::CaseInsensitive,
      occurrences: None,
      valid_search: IndexSet::new(),
    }
  }

  pub fn occurrences(&self) -> &Option<Selection> {
    &self.occurrences
  }

  pub fn hls_dirty(&self) -> bool {
    self.hls_dirty
  }

  pub fn set_hls_dirty(&mut self, is_dirty: bool) {
    self.hls_dirty = is_dirty
  }

  pub fn update_highlights(&mut self, text: &Rope, last_text: &Rope,
                           delta: &Delta<RopeInfo>) {
    // todo
    // Update search highlights for changed regions
    if self.search_string.is_some() {
      self.valid_search = self.valid_search.apply_delta(delta);
      let mut occurrences = self.occurrences.take().unwrap_or_else(Selection::new);

      // invalidate occurrences around deletion positions
      for DeltaRegion{ old_offset, new_offset, len } in delta.iter_deletions() {
        self.valid_search.delete_range(new_offset, new_offset + len);
        occurrences.delete_range(old_offset, old_offset + len, false);
      }

      occurrences = occurrences.apply_delta(delta, false, false);

      // invalidate occurrences around insert positions
      for DeltaRegion{ new_offset, len, .. } in delta.iter_inserts() {
        self.valid_search.delete_range(new_offset, new_offset + len);
        occurrences.delete_range(new_offset, new_offset + len, false);
      }

      self.occurrences = Some(occurrences);

      // update find for the whole delta (is going to only update invalid regions)
      let (iv, _) = delta.summary();
      self.update_find(text, iv.start(), iv.end(), true, false);
    }
  }

  pub fn do_find(&mut self, text: &Rope, search_string: Option<String>,
                 case_sensitive: bool) -> Value {
      if search_string.is_none() {
          self.unset();
          return Value::Null;
      }

      let search_string = search_string.unwrap();
      if search_string.len() == 0 {
          self.unset();
          return Value::Null;
      }

      self.set_find(text, &search_string, case_sensitive);

      Value::String(search_string.to_string())
  }

  /// Unsets the search and removes all highlights from the view.
  pub fn unset(&mut self) {
      self.search_string = None;
      self.occurrences = None;
      self.hls_dirty = true;
      self.valid_search.clear();
  }

  /// Sets find for the view, highlights occurrences in the current viewport
  /// and selects the first occurrence relative to the last cursor.
  fn set_find(&mut self, text: &Rope, search_string: &str,
              case_sensitive: bool) {
      // todo: this will be removed once multiple queries are supported
      let case_matching = if case_sensitive {
          CaseMatching::Exact
      } else {
          CaseMatching::CaseInsensitive
      };

      if let Some(ref s) = self.search_string {
          if s == search_string && case_matching == self.case_matching {
              // search parameters did not change
              return;
          }
      }

      self.unset();

      self.search_string = Some(search_string.to_string());
      self.case_matching = case_matching;
  }

  pub fn update_find(&mut self, text: &Rope, start: usize, end: usize, include_slop: bool,
                 stop_on_found: bool)
  {
      if self.search_string.is_none() {
          return;
      }

      let text_len = text.len();
      // extend the search by twice the string length (twice, because case matching may increase
      // the length of an occurrence)
      let slop = if include_slop { self.search_string.as_ref().unwrap().len() * 2 } else { 0 };
      let mut occurrences = self.occurrences.take().unwrap_or_else(Selection::new);
      let mut searched_until = end;
//      let mut invalidate_from = None;

      for (start, end) in self.valid_search.minus_one_range(start, end) {
        let search_string = self.search_string.as_ref().unwrap();
        let len = search_string.len();

        // expand region to be able to find occurrences around the region's edges
        let from = max(0, slop) - slop;
        let to = min(end + slop, text.len());

        // TODO: this interval might cut a unicode codepoint, make sure it is
        // aligned to codepoint boundaries.
        let text = text.subseq(Interval::new_closed_open(0, to));
        let mut cursor = Cursor::new(&text, from);

        while cursor.pos() < end {
          match find(&mut cursor, self.case_matching, &search_string) {
            Some(start) => {
              let end = start + len;

              let region = SelRegion::new(start, end);
              eprintln!("Start of occurrence {:?}, End {:?}", start, end);

              let prev_len = occurrences.len();
              let (_, e) = occurrences.add_range_distinct(region);
              // in case of ambiguous search results (e.g. search "aba" in "ababa"),
              // the search result closer to the beginning of the file wins
              if e != end {
                // Skip the search result and keep the occurrence that is closer to
                // the beginning of the file. Re-align the cursor to the kept
                // occurrence
                cursor.set(e);
                continue;
              }
              cursor.set(end);
            }
            _ => {}
          }
        }

      }
      self.occurrences = Some(occurrences);

      // commented out for simplicity
//      if let Some(invalidate_from) = invalidate_from {
//          self.valid_search.union_one_range(start, invalidate_from);
//
//          // invalidate all search results from the point of the ambiguous search result until ...
//          let is_multi_line = LinesMetric::next(self.search_string.as_ref().unwrap(), 0).is_some();
//          if is_multi_line {
//              // ... the end of the file
//              self.valid_search.delete_range(invalidate_from, text_len);
//          } else {
//              // ... the end of the line
//              let mut cursor = Cursor::new(&text, invalidate_from);
//              if let Some(end_of_line) = cursor.next::<LinesMetric>() {
//                  self.valid_search.delete_range(invalidate_from, end_of_line);
//              }
//          }
//
//          // continue with the find for the current region
//          self.update_find(text, invalidate_from, end, false, false);
//      } else {
          self.valid_search.union_one_range(start, searched_until);
          self.hls_dirty = true;
//      }
  }

  pub fn next_occurrence(&mut self, text: &Rope, reverse: bool, wrapped: bool,
                         stop_on_found: bool, allow_same: bool, sel: (usize, usize)) -> Option<SelRegion> {
    let mut next_occurrence;
    let (from, to) = if reverse != wrapped { (0, sel.0) } else { (sel.0, text.len()) };

    loop {
      next_occurrence = self.occurrences.as_ref().and_then(|occurrences| {
        if occurrences.len() == 0 {
          return None;
        }
        if wrapped { // wrap around file boundaries
          if reverse {
            occurrences.last()
          } else {
            occurrences.first()
          }
        } else {
          let ix = occurrences.search(sel.0);
          if reverse {
            ix.checked_sub(1).and_then(|i| occurrences.get(i))
          } else {
            occurrences.get(ix).and_then(|oc| {
              // if possible, the current selection should be extended, instead of
              // jumping to the next occurrence
              if oc.end == sel.1 && !allow_same {
                occurrences.get(ix+1)
              } else {
                Some(oc)
              }
            })
          }
        }
      }).cloned();

      let region = {
        let mut unsearched = self.valid_search.minus_one_range(from, to);
        if reverse { unsearched.next_back() } else { unsearched.next() }
      };
      if let Some((b, e)) = region {
        if let Some(ref occurrence) = next_occurrence {
          if (reverse && occurrence.start >= e) || (!reverse && occurrence.end <= b) {
            break;
          }
        }

        if !reverse {
          self.update_find(text, b, e, false, stop_on_found);
        } else {
          // when searching backward, the actual search isn't executed backwards, which is
          // why the search is executed in chunks
          let start = if e - b > BACKWARDS_FIND_CHUNK_SIZE {
            e - BACKWARDS_FIND_CHUNK_SIZE
          } else {
            b
          };
          self.update_find(text, start, e, false, false);
        }
      } else {
        break;
      }
    }

    next_occurrence
  }
}