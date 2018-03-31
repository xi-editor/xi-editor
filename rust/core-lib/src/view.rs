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

use std::cmp::{min,max};
use std::mem;

use serde_json::value::Value;

use xi_rope::rope::{Rope, LinesMetric, RopeInfo};
use xi_rope::delta::{Delta, DeltaRegion};
use xi_rope::tree::{Cursor, Metric};
use xi_rope::breaks::{Breaks, BreaksInfo, BreaksMetric, BreaksBaseMetric};
use xi_rope::interval::Interval;
use xi_rope::spans::Spans;
use xi_rope::find::{find, CaseMatching};

use tabs::{ViewIdentifier, DocumentCtx};
use styles::Style;
use index_set::IndexSet;
use selection::{Affinity, Selection, SelRegion};
use movement::{Movement, selection_movement};
use line_cache_shadow::{self, LineCacheShadow, RenderPlan, RenderTactic};

use linewrap;

const BACKWARDS_FIND_CHUNK_SIZE: usize = 32_768;

pub struct View {
    pub view_id: ViewIdentifier,

    /// The selection state for this view. Invariant: non-empty.
    selection: Selection,

    drag_state: Option<DragState>,

    first_line: usize,  // vertical scroll position
    height: usize,  // height of visible portion
    breaks: Option<Breaks>,
    wrap_col: usize,

    /// Front end's line cache state for this view. See the `LineCacheShadow`
    /// description for the invariant.
    lc_shadow: LineCacheShadow,

    // The occurrences, which determine the highlights, have been updated.
    hls_dirty: bool,

    /// Tracks whether or not the view has unsaved changes.
    pristine: bool,

    /// The currently active search string
    search_string: Option<String>,
    /// The case matching setting for the currently active search
    case_matching: CaseMatching,
    /// The set of all known find occurrences (highlights)
    occurrences: Option<Selection>,
    /// Set of ranges that have already been searched for the currently active search string
    valid_search: IndexSet,
}

/// State required to resolve a drag gesture into a selection.
struct DragState {
    /// All the selection regions other than the one being dragged.
    base_sel: Selection,

    /// Offset of the point where the drag started.
    offset: usize,

    /// Start of the region selected when drag was started (region is
    /// assumed to be forward).
    min: usize,

    /// End of the region selected when drag was started.
    max: usize,
}

impl View {
    pub fn new(view_id: ViewIdentifier) -> View {
        let mut selection = Selection::new();
        selection.add_region(SelRegion {
            start: 0,
            end: 0,
            horiz: None,
            affinity: Affinity::default(),
        });
        View {
            view_id: view_id.to_owned(),
            selection: selection,
            drag_state: None,
            first_line: 0,
            height: 10,
            breaks: None,
            wrap_col: 0,
            lc_shadow: LineCacheShadow::default(),
            hls_dirty: true,
            pristine: true,
            search_string: None,
            case_matching: CaseMatching::CaseInsensitive,
            occurrences: None,
            valid_search: IndexSet::new(),
        }
    }

    pub fn set_scroll(&mut self, first: usize, last: usize) {
        self.first_line = first;
        self.height = last - first;
    }

    pub fn scroll_height(&self) -> usize {
        self.height
    }

    pub fn scroll_to_cursor(&mut self, text: &Rope) {
        let end = self.sel_regions().last().unwrap().end;
        let line = self.line_of_offset(text, end);
        if line < self.first_line {
            self.first_line = line;
        } else if self.first_line + self.height <= line {
            self.first_line = line - (self.height - 1);
        }
    }

    /// Toggles a caret at the given offset.
    pub fn toggle_sel(&mut self, text: &Rope, offset: usize) {
        // We could probably reduce the cloning of selections by being clever.
        let mut selection = self.selection.clone();
        if !selection.regions_in_range(offset, offset).is_empty() {
            selection.delete_range(offset, offset, true);
            if !selection.is_empty() {
                self.drag_state = None;
                self.set_selection_raw(text, selection);
                return;
            }
        }
        self.drag_state = Some(DragState {
            base_sel: selection.clone(),
            offset: offset,
            min: offset,
            max: offset,
        });
        let region = SelRegion {
            start: offset,
            end: offset,
            horiz: None,
            affinity: Affinity::default(),
        };
        selection.add_region(region);
        self.set_selection_raw(text, selection);
    }

    /// Move the selection by the given movement. Return value is the offset of
    /// a point that should be scrolled into view.
    ///
    /// If `modify` is `true`, the selections are modified, otherwise the results
    /// of individual region movements become carets.
    pub fn do_move(&mut self, text: &Rope, movement: Movement, modify: bool)
        -> Option<usize>
    {
        self.drag_state = None;
        let new_sel = selection_movement(movement, &self.selection, self, text, modify);
        self.set_selection(text, new_sel)
    }

    /// Set the selection to a new value. Return value is the offset of a
    /// point that should be scrolled into view.
    pub fn set_selection(&mut self, text: &Rope, sel: Selection) -> Option<usize> {
        self.set_selection_raw(text, sel);
        // We somewhat arbitrarily choose the last region for setting the old-style
        // selection state, and for scrolling it into view if needed. This choice can
        // likely be improved.
        let region = self.selection.last().unwrap().clone();
        self.scroll_to_cursor(text);
        Some(region.end)
    }

    /// Sets the selection to a new value, without invalidating. Return values is
    /// the offset of a point that should be scrolled into view.
    fn set_selection_for_edit(&mut self, text: &Rope, sel: Selection) -> Option<usize> {
        self.selection = sel;
        // We somewhat arbitrarily choose the last region for setting the old-style
        // selection state, and for scrolling it into view if needed. This choice can
        // likely be improved.
        let region = self.selection.last().unwrap().clone();
        self.scroll_to_cursor(text);
        Some(region.end)
    }

    /// Sets the selection to a new value, invalidating the line cache as needed. This
    /// function does not perform any scrolling.
    fn set_selection_raw(&mut self, text: &Rope, sel: Selection) {
        self.invalidate_selection(text);
        self.selection = sel;
        self.invalidate_selection(text);
    }

    /// Invalidate the current selection. Note that we could be even more fine-grained
    /// in the case of multiple cursors, but we also want this method to be fast even
    /// when the selection is large.
    fn invalidate_selection(&mut self, text: &Rope) {
        // TODO: refine for upstream (caret appears on prev line)
        let first_line = self.line_of_offset(text, self.selection.first().unwrap().min());
        let last_line = self.line_of_offset(text, self.selection.last().unwrap().max()) + 1;
        let all_caret = self.selection.iter().all(|region| region.is_caret());
        let invalid = if all_caret {
            line_cache_shadow::CURSOR_VALID
        } else {
            line_cache_shadow::CURSOR_VALID | line_cache_shadow::STYLES_VALID
        };
        self.lc_shadow.partial_invalidate(first_line, last_line, invalid);
    }

    /// Invalidates the styles of the given range (start and end are offsets within
    /// the text).
    pub fn invalidate_styles(&mut self, text: &Rope, start: usize, end: usize) {
        let first_line = self.line_of_offset(text, start);
        let (mut last_line, last_col) = self.offset_to_line_col(text, end);
        last_line += if last_col > 0 { 1 } else { 0 };
        self.lc_shadow.partial_invalidate(first_line, last_line, line_cache_shadow::STYLES_VALID);
    }

    /// Select entire buffer.
    ///
    /// Note: unlike movement based selection, this does not scroll.
    pub fn select_all(&mut self, text: &Rope) {
        let mut selection = Selection::new();
        selection.add_region(SelRegion {
            start: 0,
            end: text.len(),
            horiz: None,
            affinity: Default::default(),
        });
        self.set_selection_raw(text, selection);
    }

    /// Starts a drag operation.
    pub fn start_drag(&mut self, offset: usize, min: usize, max: usize) {
        let base_sel = Selection::new();
        self.drag_state = Some(DragState { base_sel, offset, min, max });
    }

    /// Does a drag gesture, setting the selection from a combination of the drag
    /// state and new offset.
    pub fn do_drag(&mut self, text: &Rope, offset: usize, affinity: Affinity) -> Option<usize> {
        let new_sel = self.drag_state.as_ref().map(|drag_state| {
            let mut sel = drag_state.base_sel.clone();
            // TODO: on double or triple click, quantize offset to requested granularity.
            let (start, end) = if offset < drag_state.offset {
                (drag_state.max, min(offset, drag_state.min))
            } else {
                (drag_state.min, max(offset, drag_state.max))
            };
            let horiz = None;
            sel.add_region(SelRegion { start, end, horiz, affinity });
            sel
        });
        new_sel.and_then(|new_sel| self.set_selection(text, new_sel))
    }

    /// Returns the regions of the current selection.
    pub fn sel_regions(&self) -> &[SelRegion] {
        &self.selection
    }

    /// Collapse all selections in this view into a single caret
    pub fn collapse_selections(&mut self, text: &Rope) {
        let mut sel = self.selection.clone();
        sel.collapse();
        &self.set_selection(text, sel);
    }

    /// Determines whether the offset is in any selection (counting carets and
    /// selection edges).
    pub fn is_point_in_selection(&self, offset: usize) -> bool {
        !self.selection.regions_in_range(offset, offset).is_empty()
    }

    // Render a single line, and advance cursors to next line.
    fn render_line(&self, tab_ctx: &DocumentCtx, text: &Rope,
        start_of_line: &mut Cursor<RopeInfo>, soft_breaks: Option<&mut Cursor<BreaksInfo>>, style_spans: &Spans<Style>,
        line_num: usize) -> Value
    {
        let start_pos = start_of_line.pos();
        let pos = soft_breaks.map_or(start_of_line.next::<LinesMetric>(), |bc| {
            let pos = bc.next::<BreaksMetric>();
            // if using breaks update cursor
            if let Some(pos) = pos { start_of_line.set(pos) }
            pos
        }).unwrap_or(text.len());

        let l_str = text.slice_to_string(start_pos, pos);
        let mut cursors = Vec::new();
        let mut selections = Vec::new();
        for region in self.selection.regions_in_range(start_pos, pos) {
            // cursor
            let c = region.end;
            if (c > start_pos && c < pos) ||
                (!region.is_upstream() && c == start_pos) ||
                (region.is_upstream() && c == pos) ||
                (c == pos && c == text.len() && self.line_of_offset(text, c) == line_num)
            {
                cursors.push(c - start_pos);
            }

            // selection with interior
            let sel_start_ix = clamp(region.min(), start_pos, pos) - start_pos;
            let sel_end_ix = clamp(region.max(), start_pos, pos) - start_pos;
            if sel_end_ix > sel_start_ix {
                selections.push((sel_start_ix, sel_end_ix));
            }
        }

        let mut hls = Vec::new();
        if let Some(ref occurrences) = self.occurrences {
            for region in occurrences.regions_in_range(start_pos, pos) {
                let sel_start_ix = clamp(region.min(), start_pos, pos) - start_pos;
                let sel_end_ix = clamp(region.max(), start_pos, pos) - start_pos;
                if sel_end_ix > sel_start_ix {
                    hls.push((sel_start_ix, sel_end_ix));
                }
            }
        }

        let styles = self.render_styles(tab_ctx, start_pos, pos, &selections, &hls, style_spans);

        let mut result = json!({
            "text": &l_str,
            "styles": styles,
        });

        if !cursors.is_empty() {
            result["cursor"] = json!(cursors);
        }
        result
    }

    pub fn render_styles(&self, tab_ctx: &DocumentCtx, start: usize, end: usize,
        sel: &[(usize, usize)], hls: &[(usize, usize)], style_spans: &Spans<Style>) -> Vec<isize>
    {
        let mut rendered_styles = Vec::new();
        let style_spans = style_spans.subseq(Interval::new_closed_open(start, end));

        let mut ix = 0;
        for &(sel_start, sel_end) in sel {
            rendered_styles.push((sel_start as isize) - ix);
            rendered_styles.push(sel_end as isize - sel_start as isize);
            rendered_styles.push(0);
            ix = sel_end as isize;
        }
        for &(sel_start, sel_end) in hls {
            rendered_styles.push((sel_start as isize) - ix);
            rendered_styles.push(sel_end as isize - sel_start as isize);
            rendered_styles.push(1);
            ix = sel_end as isize;
        }
        for (iv, style) in style_spans.iter() {
            let style_id = tab_ctx.get_style_id(&style);
            rendered_styles.push((iv.start() as isize) - ix);
            rendered_styles.push(iv.end() as isize - iv.start() as isize);
            rendered_styles.push(style_id as isize);
            ix = iv.end() as isize;
        }
        rendered_styles
    }

    fn build_update_op(&self, op: &str, lines: Option<Vec<Value>>, n: usize) -> Value {
        let mut update = json!({
            "op": op,
            "n": n,
        });

        if let Some(lines) = lines {
            update["lines"] = json!(lines);
        }

        update
    }

    fn send_update_for_plan(&mut self, text: &Rope, tab_ctx: &DocumentCtx,
        style_spans: &Spans<Style>, plan: &RenderPlan)
    {
        if !self.lc_shadow.needs_render(plan) { return; }

        let mut b = line_cache_shadow::Builder::new();
        let mut ops = Vec::new();
        let mut line_num = 0;  // tracks old line cache

        // Note: if we weren't doing mutable update_find_for_lines in the loop, we
        // could just borrow self.lc_shadow instead of doing this.
        let lc_shadow = mem::replace(&mut self.lc_shadow, LineCacheShadow::default());
        for seg in lc_shadow.iter_with_plan(plan) {
            match seg.tactic {
                RenderTactic::Discard => {
                    ops.push(self.build_update_op("invalidate", None, seg.n));
                    b.add_span(seg.n, 0, 0);
                }
                RenderTactic::Preserve => {
                    // TODO: in the case where it's ALL_VALID & !CURSOR_VALID, and cursors
                    // are empty, could send update removing the cursor.
                    if seg.validity == line_cache_shadow::ALL_VALID {
                        let n_skip = seg.their_line_num - line_num;
                        if n_skip > 0 {
                            ops.push(self.build_update_op("skip", None, n_skip));
                        }
                        ops.push(self.build_update_op("copy", None, seg.n));
                        b.add_span(seg.n, seg.our_line_num, line_cache_shadow::ALL_VALID);
                        line_num = seg.their_line_num + seg.n;
                    } else {
                        ops.push(self.build_update_op("invalidate", None, seg.n));
                        b.add_span(seg.n, 0, 0);
                    }
                }
                RenderTactic::Render => {
                    // TODO: update (rather than re-render) in cases of text valid
                    if seg.validity == line_cache_shadow::ALL_VALID {
                        let n_skip = seg.their_line_num - line_num;
                        if n_skip > 0 {
                            ops.push(self.build_update_op("skip", None, n_skip));
                        }
                        ops.push(self.build_update_op("copy", None, seg.n));
                        b.add_span(seg.n, seg.our_line_num, line_cache_shadow::ALL_VALID);
                        line_num = seg.their_line_num + seg.n;
                    } else {
                        let start_line = seg.our_line_num;
                        let end_line = start_line + seg.n;
                        if self.hls_dirty {
                            self.update_find_for_lines(text, start_line, end_line);
                        }
                        let offset = self.offset_of_line(text, start_line);
                        let mut line_cursor = Cursor::new(text, offset);
                        let mut soft_breaks = self.breaks.as_ref().map(|breaks|
                            Cursor::new(breaks, offset));
                        let mut rendered_lines = Vec::new();
                        for line_num in start_line..end_line {
                            rendered_lines.push(self.render_line(tab_ctx, text,
                                &mut line_cursor, soft_breaks.as_mut(), style_spans, line_num));
                        }
                        ops.push(self.build_update_op("ins", Some(rendered_lines), seg.n));
                        b.add_span(seg.n, seg.our_line_num, line_cache_shadow::ALL_VALID);
                    }
                }
            }
        }
        let params = json!({
            "ops": ops,
            "pristine": self.pristine,
        });
        tab_ctx.update_view(self.view_id, &params);
        self.lc_shadow = b.build();
        self.hls_dirty = false;
    }

    // Update front-end with any changes to view since the last time sent.
    pub fn render_if_dirty(&mut self, text: &Rope, tab_ctx: &DocumentCtx,
        style_spans: &Spans<Style>)
    {
        let height = self.line_of_offset(text, text.len()) + 1;
        let plan = RenderPlan::create(height, self.first_line, self.height);
        self.send_update_for_plan(text, tab_ctx, style_spans, &plan);
    }

    // Send the requested lines even if they're outside the current scroll region.
    pub fn request_lines(&mut self, text: &Rope, tab_ctx: &DocumentCtx, style_spans: &Spans<Style>,
        first_line: usize, last_line: usize) {
        let height = self.line_of_offset(text, text.len()) + 1;
        let mut plan = RenderPlan::create(height, self.first_line, self.height);
        plan.request_lines(first_line, last_line);
        self.send_update_for_plan(text, tab_ctx, style_spans, &plan);
    }

    /// Invalidates front-end's entire line cache, forcing a full render at the next
    /// update cycle. This should be a last resort, updates should generally cause
    /// finer grain invalidation.
    pub fn set_dirty(&mut self, text: &Rope) {
        let height = self.line_of_offset(text, text.len()) + 1;
        let mut b = line_cache_shadow::Builder::new();
        b.add_span(height, 0, 0);
        b.set_dirty(true);
        self.lc_shadow = b.build();
    }

    // How should we count "column"? Valid choices include:
    // * Unicode codepoints
    // * grapheme clusters
    // * Unicode width (so CJK counts as 2)
    // * Actual measurement in text layout
    // * Code units in some encoding
    //
    // Of course, all these are identical for ASCII. For now we use UTF-8 code units
    // for simplicity.

    pub fn offset_to_line_col(&self, text: &Rope, offset: usize) -> (usize, usize) {
        let line = self.line_of_offset(text, offset);
        (line, offset - self.offset_of_line(text, line))
    }

    pub fn line_col_to_offset(&self, text: &Rope, line: usize, col: usize) -> usize {
        let mut offset = self.offset_of_line(text, line).saturating_add(col);
        if offset >= text.len() {
            offset = text.len();
            if self.line_of_offset(text, offset) <= line {
                return offset;
            }
        } else {
            // Snap to grapheme cluster boundary
            offset = text.prev_grapheme_offset(offset + 1).unwrap();
        }

        // clamp to end of line
        let next_line_offset = self.offset_of_line(text, line + 1);
        if offset >= next_line_offset {
            if let Some(prev) = text.prev_grapheme_offset(next_line_offset) {
                offset = prev;
            }
        }
        offset
    }

    // use own breaks if present, or text if not (no line wrapping)

    /// Returns the visible line number containing the given offset.
    pub fn line_of_offset(&self, text: &Rope, offset: usize) -> usize {
        match self.breaks {
            Some(ref breaks) => {
                breaks.convert_metrics::<BreaksBaseMetric, BreaksMetric>(offset)
            }
            None => text.line_of_offset(offset)
        }
    }

    /// Returns the byte offset corresponding to the line `line`.
    pub fn offset_of_line(&self, text: &Rope, line: usize) -> usize {
        match self.breaks {
            Some(ref breaks) => {
                breaks.convert_metrics::<BreaksMetric, BreaksBaseMetric>(line)
            }
            None => {
                // sanitize input
                let line = line.min(text.measure::<LinesMetric>() + 1);
                text.offset_of_line(line)
            }
        }
    }

    pub fn rewrap(&mut self, text: &Rope, wrap_col: usize) {
        if wrap_col > 0 {
            self.breaks = Some(linewrap::linewrap(text, wrap_col));
            self.wrap_col = wrap_col;
        } else {
            self.breaks = None
        }
    }

    /// Updates the view after the text has been modified by the given `delta`.
    /// This method is responsible for updating the cursors, and also for
    /// recomputing line wraps.
    ///
    /// Return value is a location of a point that should be scrolled into view.
    pub fn after_edit(&mut self, text: &Rope, last_text: &Rope, delta: &Delta<RopeInfo>,
        pristine: bool, keep_selections: bool) -> Option<usize>
    {
        let (iv, new_len) = delta.summary();
        if let Some(breaks) = self.breaks.as_mut() {
            linewrap::rewrap(breaks, text, iv, new_len, self.wrap_col);
        }
        if self.breaks.is_some() {
            // TODO: finer grain invalidation for the line wrapping, needs info
            // about what wrapped.
            self.set_dirty(text);
        } else {
            let start = self.line_of_offset(last_text, iv.start());
            let end = self.line_of_offset(last_text, iv.end()) + 1;
            let new_end = self.line_of_offset(text, iv.start() + new_len) + 1;
            self.lc_shadow.edit(start, end, new_end - start);
        }
        self.pristine = pristine;
        // Any edit cancels a drag. This is good behavior for edits initiated through
        // the front-end, but perhaps not for async edits.
        self.drag_state = None;

        // Update search highlights for changed regions
        if self.search_string.is_some() {
            self.valid_search = self.valid_search.apply_delta(delta);
            let mut occurrences = self.occurrences.take().unwrap_or_else(|| Selection::new());

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

        // Note: for committing plugin edits, we probably want to know the priority
        // of the delta so we can set the cursor before or after the edit, as needed.
        let new_sel = self.selection.apply_delta(delta, true, keep_selections);
        self.set_selection_for_edit(text, new_sel)
    }

    /// Call to mark view as pristine. Used after a buffer is saved.
    pub fn set_pristine(&mut self) {
        self.pristine = true;
    }

    /// Unsets the search and removes all highlights from the view.
    pub fn unset_find(&mut self, text: &Rope) {
        self.search_string = None;
        self.occurrences = None;
        self.hls_dirty = true;
        // TODO: finer grained invalidation
        self.set_dirty(text);
        self.valid_search.clear();
    }

    /// Sets find for the view, highlights occurrences in the current viewport and selects the first
    /// occurrence relative to the last cursor.
    pub fn set_find(&mut self, text: &Rope, search_string: &str, case_sensitive: bool) {
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

        self.unset_find(text);

        self.search_string = Some(search_string.to_string());
        self.case_matching = case_matching;
    }

    fn update_find_for_lines(&mut self, text: &Rope, first_line: usize, last_line: usize) {
        if self.search_string.is_none() {
            return;
        }
        let start = self.offset_of_line(text, first_line);
        let end = self.offset_of_line(text, last_line);
        self.update_find(text, start, end, true, false);
    }

    fn update_find(&mut self, text: &Rope, start: usize, end: usize, include_slop: bool,
                   stop_on_found: bool)
    {
        if self.search_string.is_none() {
            return;
        }

        let text_len = text.len();
        // extend the search by twice the string length (twice, because case matching may increase
        // the length of an occurrence)
        let slop = if include_slop { self.search_string.as_ref().unwrap().len() * 2 } else { 0 };
        let mut occurrences = self.occurrences.take().unwrap_or_else(|| Selection::new());
        let mut searched_until = end;
        let mut invalidate_from = None;

        for (start, end) in self.valid_search.minus_one_range(start, end) {
            let search_string = self.search_string.as_ref().unwrap();
            let len = search_string.len();

            // expand region to be able to find occurrences around the region's edges
            let from = max(start, slop) - slop;
            let to = min(end + slop, text.len());

            // TODO: this interval might cut a unicode codepoint, make sure it is
            // aligned to codepoint boundaries.
            let text = text.subseq(Interval::new_closed_open(0, to));
            let mut cursor = Cursor::new(&text, from);

            loop {
                match find(&mut cursor, self.case_matching, &search_string) {
                    Some(start) => {
                        let end = start + len;

                        let region = SelRegion {
                            start: start,
                            end: end,
                            horiz: None,
                            affinity: Affinity::default(),
                        };
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

                        // add_range_distinct() above removes ambiguous regions after the added
                        // region, if something has been deleted, everything thereafter is
                        // invalidated
                        if occurrences.len() != prev_len + 1 {
                            invalidate_from = Some(end);
                            occurrences.delete_range(end, text_len, false);
                            break;
                        }

                        if stop_on_found {
                            searched_until = end;
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
        self.occurrences = Some(occurrences);
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
            self.update_find(text, invalidate_from, end, false, false);
        } else {
            self.valid_search.union_one_range(start, searched_until);
            self.hls_dirty = true;
        }
    }

    /// Select the next occurrence relative to the last cursor. `reverse` determines whether the
    /// next occurrence before (`true`) or after (`false`) the last cursor is selected. `wrapped`
    /// indicates a search for the next occurrence past the end of the file. `stop_on_found`
    /// determines whether the search should stop at the first found occurrence (does only apply
    /// to forward search, i.e. reverse = false). If `allow_same` is set to `true` the current
    /// selection is considered a valid next occurrence.
    pub fn select_next_occurrence(&mut self, text: &Rope, reverse: bool, wrapped: bool,
                                  stop_on_found: bool, allow_same: bool) -> Option<usize>
    {
        if self.search_string.is_none() {
            return None;
        }

        let sel = match self.sel_regions().last() {
            Some(sel) => (sel.min(), sel.max()),
            None => return None
        };

        let (from, to) = if reverse != wrapped { (0, sel.0) } else { (sel.0, text.len()) };
        let mut next_occurrence;

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
            }).map(|o| o.clone());

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

        if let Some(occurrence) = next_occurrence {
            let mut selection = Selection::new();
            selection.add_region(occurrence);
            self.set_selection(text, selection)
        } else {
            None
        }
    }

    /// Generate line breaks based on width measurement. Currently batch-mode,
    /// and currently in a debugging state.
    pub fn wrap_width(&mut self, text: &Rope, tab_ctx: &DocumentCtx,
        style_spans: &Spans<Style>)
    {
        self.breaks = Some(linewrap::linewrap_width(text, style_spans, tab_ctx, 500.0));
    }
}

// utility function to clamp a value within the given range
fn clamp(x: usize, min: usize, max: usize) -> usize {
    if x < min {
        min
    } else if x < max {
        x
    } else {
        max
    }
}
