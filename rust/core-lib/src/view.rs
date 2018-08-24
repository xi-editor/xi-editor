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

use std::cmp::{min,max};
use std::cell::RefCell;
use std::ops::Range;

use serde_json::Value;

use xi_rope::rope::{Rope, LinesMetric, RopeInfo};
use xi_rope::delta::Delta;
use xi_rope::tree::Cursor;
use xi_rope::breaks::{Breaks, BreaksInfo, BreaksMetric, BreaksBaseMetric};
use xi_rope::interval::Interval;
use xi_rope::spans::Spans;
use xi_trace::trace_block;
use client::Client;
use edit_types::ViewEvent;
use line_cache_shadow::{self, LineCacheShadow, RenderPlan, RenderTactic};
use movement::{Movement, region_movement, selection_movement};
use rpc::{GestureType, MouseAction, SelectionModifier};
use styles::{Style, ThemeStyleMap};
use selection::{Affinity, Selection, SelRegion};
use tabs::{ViewId, BufferId};
use width_cache::WidthCache;
use word_boundaries::WordCursor;
use find::Find;
use linewrap;
use internal::find::FindStatus;

type StyleMap = RefCell<ThemeStyleMap>;


/// A flag used to indicate when legacy actions should modify selections
const FLAG_SELECT: u64 = 2;

pub struct View {
    view_id: ViewId,
    buffer_id: BufferId,

    /// Tracks whether this view has been scheduled to render.
    /// We attempt to reduce duplicate renders by setting a small timeout
    /// after an edit is applied, to allow batching with any plugin updates.
    pending_render: bool,
    size: Size,
    /// The selection state for this view. Invariant: non-empty.
    selection: Selection,

    drag_state: Option<DragState>,

    /// vertical scroll position
    first_line: usize,
    /// height of visible portion
    height: usize,
    breaks: Option<Breaks>,
    wrap_col: WrapWidth,

    /// Front end's line cache state for this view. See the `LineCacheShadow`
    /// description for the invariant.
    lc_shadow: LineCacheShadow,

    /// New offset to be scrolled into position after an edit.
    scroll_to: Option<usize>,

    /// The state for finding text for this view.
    /// Each instance represents a separate search query.
    find: Vec<Find>,

    /// Tracks whether there has been changes in find results or find parameters.
    /// This is used to determined whether FindStatus should be sent to the frontend.
    find_changed: FindStatusChange,

    /// Tracks whether find highlights should be rendered.
    /// Highlights are only rendered when search dialog is open.
    highlight_find: bool,

    /// The state for replacing matches for this view.
    replace: Option<Replace>,

    /// Tracks whether the replacement string or replace parameters changed.
    replace_changed: bool,
}

/// Indicates what changed in the find state.
#[derive(PartialEq, Debug)]
enum FindStatusChange {
    /// None of the find parameters or number of matches changed.
    None,

    /// Find parameters and number of matches changed.
    All,

    /// Only number of matches changed
    Matches
}

/// Contains replacement string and replace options.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize, Clone)]
pub struct Replace {
    /// Replacement string.
    pub chars: String,
    pub preserve_case: bool
}

/// A size, in pixel units (not display pixels).
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

/// The visual width of the buffer for the purpose of word wrapping.
enum WrapWidth {
    /// No wrapping in effect.
    None,

    /// Width in bytes (utf-8 code units).
    ///
    /// Only works well for ASCII, will probably not be maintained long-term.
    Bytes(usize),

    /// Width in px units, requiring measurement by the front-end.
    Width(f64),
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
    pub fn new(view_id: ViewId, buffer_id: BufferId) -> View {
        View {
            view_id: view_id,
            buffer_id: buffer_id,
            pending_render: false,
            selection: SelRegion::caret(0).into(),
            scroll_to: Some(0),
            size: Size::default(),
            drag_state: None,
            first_line: 0,
            height: 10,
            breaks: None,
            wrap_col: WrapWidth::None,
            lc_shadow: LineCacheShadow::default(),
            find: Vec::new(),
            find_changed: FindStatusChange::None,
            highlight_find: false,
            replace: None,
            replace_changed: false,
        }
    }

    pub(crate) fn get_buffer_id(&self) -> BufferId {
        self.buffer_id
    }

    pub(crate) fn get_view_id(&self) -> ViewId {
        self.view_id
    }

    pub(crate) fn get_replace(&self) -> Option<Replace> {
        self.replace.clone()
    }

    pub(crate) fn set_has_pending_render(&mut self, pending: bool) {
        self.pending_render = pending
    }

    pub(crate) fn has_pending_render(&self) -> bool {
        self.pending_render
    }

    pub(crate) fn do_edit(&mut self, text: &Rope, cmd: ViewEvent) {
        use self::ViewEvent::*;
        match cmd {
            Move(movement) => self.do_move(text, movement, false),
            ModifySelection(movement) => self.do_move(text, movement, true),
            SelectAll => self.select_all(text),
            Scroll(range) => self.set_scroll(range.first, range.last),
            AddSelectionAbove =>
                self.add_selection_by_movement(text, Movement::Up),
            AddSelectionBelow =>
                self.add_selection_by_movement(text, Movement::Down),
            Gesture { line, col, ty } =>
                self.do_gesture(text, line, col, ty),
            GotoLine { line } => self.goto_line(text, line),
            Find { chars, case_sensitive, regex, whole_words } =>
                self.do_find(text, chars, case_sensitive, regex, whole_words),
            FindNext { wrap_around, allow_same, modify_selection } =>
                self.do_find_next(text, false, wrap_around, allow_same, &modify_selection),
            FindPrevious { wrap_around, allow_same, modify_selection } =>
                self.do_find_next(text, true, wrap_around, allow_same, &modify_selection),
            FindAll => self.do_find_all(text),
            Click(MouseAction { line, column, flags, click_count }) => {
                // Deprecated (kept for client compatibility):
                // should be removed in favor of do_gesture
                warn!("Usage of click is deprecated; use do_gesture");
                if (flags & FLAG_SELECT) != 0 {
                    self.do_gesture(text, line, column, GestureType::RangeSelect)
                } else if click_count == Some(2) {
                    self.do_gesture(text, line, column, GestureType::WordSelect)
                } else if click_count == Some(3) {
                    self.do_gesture(text, line, column, GestureType::LineSelect)
                } else {
                    self.do_gesture(text, line, column, GestureType::PointSelect)
                }
            }
            Drag(MouseAction { line, column, .. }) =>
                self.do_drag(text, line, column, Affinity::default()),
            Cancel => self.do_cancel(text),
            HighlightFind { visible } => {
                self.highlight_find = visible;
                self.find_changed = FindStatusChange::All;
                self.set_dirty(text);
            },
            SelectionForFind { case_sensitive } =>
                self.do_selection_for_find(text, case_sensitive),
            Replace { chars, preserve_case } =>
                self.do_set_replace(chars, preserve_case),
            SelectionForReplace => self.do_selection_for_replace(text),
            SelectionIntoLines => self.do_split_selection_into_lines(text),
        }
    }

    fn do_gesture(&mut self, text: &Rope, line: u64, col: u64, ty: GestureType) {
        let line = line as usize;
        let col = col as usize;
        let offset = self.line_col_to_offset(text, line, col);
        match ty {
            GestureType::PointSelect => {
                self.set_selection(text, SelRegion::caret(offset));
                self.start_drag(offset, offset, offset);
            },
            GestureType::RangeSelect => self.select_range(text, offset),
            GestureType::ToggleSel => self.toggle_sel(text, offset),
            GestureType::LineSelect =>
                self.select_line(text, offset, line, false),
            GestureType::WordSelect =>
                self.select_word(text, offset, false),
            GestureType::MultiLineSelect =>
                self.select_line(text, offset, line, true),
            GestureType::MultiWordSelect =>
                self.select_word(text, offset, true)
        }
    }

    fn do_cancel(&mut self, text: &Rope) {
        // if we have active find highlights, we don't collapse selections
        if self.find.is_empty() {
            self.collapse_selections(text);
        } else {
            self.unset_find();
        }
    }

    pub(crate) fn unset_find(&mut self) {
        for mut find in self.find.iter_mut() {
            find.unset();
        }
        self.find.clear();
    }

    fn goto_line(&mut self, text: &Rope, line: u64) {
        let offset = self.line_col_to_offset(text, line as usize, 0);
        self.set_selection(text, SelRegion::caret(offset));
    }

    pub fn set_size(&mut self, size: Size) {
        self.size = size;
    }

    pub fn set_scroll(&mut self, first: i64, last: i64) {
        let first = max(first, 0) as usize;
        let last = max(last, 0) as usize;
        self.first_line = first;
        self.height = last - first;
    }

    pub fn scroll_height(&self) -> usize {
        self.height
    }

    fn scroll_to_cursor(&mut self, text: &Rope) {
        let end = self.sel_regions().last().unwrap().end;
        let line = self.line_of_offset(text, end);
        if line < self.first_line {
            self.first_line = line;
        } else if self.first_line + self.height <= line {
            self.first_line = line - (self.height - 1);
        }
        // We somewhat arbitrarily choose the last region for setting the old-style
        // selection state, and for scrolling it into view if needed. This choice can
        // likely be improved.
        self.scroll_to = Some(end);
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
            offset,
            min: offset,
            max: offset,
        });
        let region = SelRegion::caret(offset);
        selection.add_region(region);
        self.set_selection_raw(text, selection);
    }

    /// Move the selection by the given movement. Return value is the offset of
    /// a point that should be scrolled into view.
    ///
    /// If `modify` is `true`, the selections are modified, otherwise the results
    /// of individual region movements become carets.
    pub fn do_move(&mut self, text: &Rope, movement: Movement, modify: bool) {
        self.drag_state = None;
        let new_sel = selection_movement(movement, &self.selection,
                                         self, text, modify);
        self.set_selection(text, new_sel);
    }

    /// Set the selection to a new value.
    pub fn set_selection<S: Into<Selection>>(&mut self, text: &Rope, sel: S) {
        self.set_selection_raw(text, sel.into());
        self.scroll_to_cursor(text);
    }

    /// Sets the selection to a new value, without invalidating.
    fn set_selection_for_edit(&mut self, text: &Rope, sel: Selection) {
        self.selection = sel;
        self.scroll_to_cursor(text);
    }

    /// Sets the selection to a new value, invalidating the line cache as needed.
    /// This function does not perform any scrolling.
    fn set_selection_raw(&mut self, text: &Rope, sel: Selection) {
        self.invalidate_selection(text);
        self.selection = sel;
        self.invalidate_selection(text);
    }

    /// Invalidate the current selection. Note that we could be even more
    /// fine-grained in the case of multiple cursors, but we also want this
    /// method to be fast even when the selection is large.
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

    fn add_selection_by_movement(&mut self, text: &Rope, movement: Movement) {
        let mut sel = Selection::new();
        for &region in self.sel_regions() {
            sel.add_region(region);
            let new_region = region_movement(movement, region, self,
                                             &text, false);
            sel.add_region(new_region);
        }
        self.set_selection(text, sel);
    }

    // TODO: insert from keyboard or input method shouldn't break undo group,
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
        let selection = SelRegion::new(0, text.len()).into();
        self.set_selection_raw(text, selection);
    }

    /// Selects a specific range (eg. when the user performs SHIFT + click).
    pub fn select_range(&mut self, text: &Rope, offset: usize) {
        if !self.is_point_in_selection(offset) {
            let sel = {
                let (last, rest) = self.sel_regions().split_last().unwrap();
                let mut sel = Selection::new();
                for &region in rest {
                    sel.add_region(region);
                }
                // TODO: small nit, merged region should be backward if end < start.
                // This could be done by explicitly overriding, or by tweaking the
                // merge logic.
                sel.add_region(SelRegion::new(last.start, offset));
                sel
            };
            self.set_selection(text, sel);
            self.start_drag(offset, offset, offset);
        }
    }

    /// Selects the given region and supports multi selection.
    fn select_region(&mut self, text: &Rope, offset: usize, region: SelRegion, multi_select: bool) {
        let mut selection = match multi_select {
            true => self.selection.clone(),
            false => Selection::new(),
        };

        selection.add_region(region);
        self.set_selection(text, selection);

        self.start_drag(offset, region.start, region.end);
    }

    /// Selects an entire word and supports multi selection.
    pub fn select_word(&mut self, text: &Rope, offset: usize, multi_select: bool) {
        let (start, end) = {
            let mut word_cursor = WordCursor::new(text, offset);
            word_cursor.select_word()
        };

        self.select_region(text, offset, SelRegion::new(start, end), multi_select);
    }

    /// Selects an entire line and supports multi selection.
    pub fn select_line(&mut self, text: &Rope, offset: usize, line: usize, multi_select: bool) {
        let start = self.line_col_to_offset(text, line, 0);
        let end = self.line_col_to_offset(text, line + 1, 0);

        self.select_region(text, offset, SelRegion::new(start, end), multi_select);
    }

    /// Splits current selections into lines.
    fn do_split_selection_into_lines(&mut self, text: &Rope) {
        let mut selection = Selection::new();

        for region in self.selection.iter() {
            if region.is_caret() {
                selection.add_region(SelRegion::caret(region.max()));
            } else {
                let mut cursor = Cursor::new(&text, region.min());

                while cursor.pos() < region.max() {
                    let sel_start = cursor.pos();
                    let end_of_line = match cursor.next::<LinesMetric>() {
                        Some(end) if end >= region.max() => max(0, region.max() - 1),
                        Some(end) => max(0, end - 1),
                        None if cursor.pos() == text.len() => cursor.pos(),
                        _ => break
                    };

                    selection.add_region(SelRegion::new(sel_start, end_of_line));
                }
            }
        }

        self.set_selection_raw(text, selection);
    }

    /// Starts a drag operation.
    pub fn start_drag(&mut self, offset: usize, min: usize, max: usize) {
        let base_sel = Selection::new();
        self.drag_state = Some(DragState { base_sel, offset, min, max });
    }

    /// Does a drag gesture, setting the selection from a combination of the drag
    /// state and new offset.
    fn do_drag(&mut self, text: &Rope, line: u64, col: u64, affinity: Affinity) {
        let offset = self.line_col_to_offset(text, line as usize, col as usize);
        let new_sel = self.drag_state.as_ref().map(|drag_state| {
            let mut sel = drag_state.base_sel.clone();
            // TODO: on double or triple click, quantize offset to requested granularity.
            let (start, end) = if offset < drag_state.offset {
                (drag_state.max, min(offset, drag_state.min))
            } else {
                (drag_state.min, max(offset, drag_state.max))
            };
            let horiz = None;
            sel.add_region(
                SelRegion::new(start, end)
                    .with_horiz(horiz)
                    .with_affinity(affinity)
            );
            sel
        });

        if let Some(sel) = new_sel {
            self.set_selection(text, sel);
        }
    }

    /// Returns the regions of the current selection.
    pub fn sel_regions(&self) -> &[SelRegion] {
        &self.selection
    }

    /// Collapse all selections in this view into a single caret
    pub fn collapse_selections(&mut self, text: &Rope) {
        let mut sel = self.selection.clone();
        sel.collapse();
        self.set_selection(text, sel);
    }

    /// Determines whether the offset is in any selection (counting carets and
    /// selection edges).
    pub fn is_point_in_selection(&self, offset: usize) -> bool {
        !self.selection.regions_in_range(offset, offset).is_empty()
    }

    // Render a single line, and advance cursors to next line.
    fn render_line(&self, client: &Client, styles: &StyleMap,
                   text: &Rope, start_of_line: &mut Cursor<RopeInfo>,
                   soft_breaks: Option<&mut Cursor<BreaksInfo>>,
                   style_spans: &Spans<Style>, line_num: usize) -> Value
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

        if self.highlight_find {
            for find in self.find.iter() {
                for region in find.occurrences().regions_in_range(start_pos, pos) {
                    let sel_start_ix = clamp(region.min(), start_pos, pos) - start_pos;
                    let sel_end_ix = clamp(region.max(), start_pos, pos) - start_pos;
                    if sel_end_ix > sel_start_ix {
                        hls.push((sel_start_ix, sel_end_ix));
                    }
                }
            }
        }

        let styles = self.render_styles(client, styles, start_pos, pos,
                                        &selections, &hls, style_spans);

        let mut result = json!({
            "text": &l_str,
            "styles": styles,
        });

        if !cursors.is_empty() {
            result["cursor"] = json!(cursors);
        }
        result
    }

    pub fn render_styles(&self, client: &Client, styles: &StyleMap,
                         start: usize, end: usize, sel: &[(usize, usize)],
                         hls: &[(usize, usize)],
                         style_spans: &Spans<Style>) -> Vec<isize>
    {
        let mut rendered_styles = Vec::new();
        let style_spans = style_spans.subseq(Interval::new_closed_open(start, end));

        let mut ix = 0;
        // we add the special find highlights (1) and selection (0) styles first.
        // We add selection after find because we want it to be preferred if the
        // same span exists in both sets (as when there is an active selection)
        for &(sel_start, sel_end) in hls {
            rendered_styles.push((sel_start as isize) - ix);
            rendered_styles.push(sel_end as isize - sel_start as isize);
            rendered_styles.push(1);
            ix = sel_end as isize;
        }
        for &(sel_start, sel_end) in sel {
            rendered_styles.push((sel_start as isize) - ix);
            rendered_styles.push(sel_end as isize - sel_start as isize);
            rendered_styles.push(0);
            ix = sel_end as isize;
        }
        for (iv, style) in style_spans.iter() {
            let style_id = self.get_or_def_style_id(client, styles, &style);
            rendered_styles.push((iv.start() as isize) - ix);
            rendered_styles.push(iv.end() as isize - iv.start() as isize);
            rendered_styles.push(style_id as isize);
            ix = iv.end() as isize;
        }
        rendered_styles
    }

    fn get_or_def_style_id(&self, client: &Client, style_map: &StyleMap,
                           style: &Style) -> usize {
        let mut style_map = style_map.borrow_mut();
        if let Some(ix) = style_map.lookup(style) {
            return ix;
        }
        let ix = style_map.add(style);
        let style = style_map.merge_with_default(style);
        client.def_style(&style.to_json(ix));
        ix
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

    fn send_update_for_plan(&mut self, text: &Rope, client: &Client,
                            styles: &StyleMap, style_spans: &Spans<Style>,
                            plan: &RenderPlan, pristine: bool)
    {
        if !self.lc_shadow.needs_render(plan) { return; }

        // send updated find status only if there have been changes
        if self.find_changed != FindStatusChange::None {
            let matches_only = self.find_changed == FindStatusChange::Matches;
            client.find_status(self.view_id, &json!(self.find_status(matches_only)));
        }

        // send updated replace status if changed
        if self.replace_changed {
            if let Some(replace) = self.get_replace() {
                client.replace_status(self.view_id, &json!(replace))
            }
        }

        let mut b = line_cache_shadow::Builder::new();
        let mut ops = Vec::new();
        let mut line_num = 0;  // tracks old line cache

        for seg in self.lc_shadow.iter_with_plan(plan) {
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

                        let offset = self.offset_of_line(text, start_line);
                        let mut line_cursor = Cursor::new(text, offset);
                        let mut soft_breaks = self.breaks.as_ref().map(|breaks|
                            Cursor::new(breaks, offset));
                        let mut rendered_lines = Vec::new();
                        for line_num in start_line..end_line {
                            let line = self.render_line(client, styles, text,
                                                        &mut line_cursor,
                                                        soft_breaks.as_mut(),
                                                        style_spans, line_num);
                            rendered_lines.push(line);
                        }
                        ops.push(self.build_update_op("ins", Some(rendered_lines), seg.n));
                        b.add_span(seg.n, seg.our_line_num, line_cache_shadow::ALL_VALID);
                    }
                }
            }
        }
        let params = json!({
            "ops": ops,
            "pristine": pristine,
        });

        client.update_view(self.view_id, &params);
        self.lc_shadow = b.build();
        for find in &mut self.find {
            find.set_hls_dirty(false)
        }
    }

    /// Determines the current number of find results and search parameters to send them to
    /// the frontend.
    pub fn find_status(&mut self, matches_only: bool) -> Vec<FindStatus> {
        self.find_changed = FindStatusChange::None;

        self.find.iter().map(|find| {
            find.find_status(matches_only)
        }).collect::<Vec<FindStatus>>()
    }

    /// Update front-end with any changes to view since the last time sent.
    /// The `pristine` argument indicates whether or not the buffer has
    /// unsaved changes.
    pub fn render_if_dirty(&mut self, text: &Rope, client: &Client,
                           styles: &StyleMap, style_spans: &Spans<Style>,
                           pristine: bool)
    {
        let height = self.line_of_offset(text, text.len()) + 1;
        let plan = RenderPlan::create(height, self.first_line, self.height);
        self.send_update_for_plan(text, client, styles,
                                  style_spans, &plan, pristine);
        if let Some(new_scroll_pos) = self.scroll_to.take() {
            let (line, col) = self.offset_to_line_col(text, new_scroll_pos);
            client.scroll_to(self.view_id, line, col);
        }
    }

    // Send the requested lines even if they're outside the current scroll region.
    pub fn request_lines(&mut self, text: &Rope, client: &Client,
                         styles: &StyleMap, style_spans: &Spans<Style>,
                         first_line: usize, last_line: usize, pristine: bool) {
        let height = self.line_of_offset(text, text.len()) + 1;
        let mut plan = RenderPlan::create(height, self.first_line, self.height);
        plan.request_lines(first_line, last_line);
        self.send_update_for_plan(text, client, styles,
                                  style_spans, &plan, pristine);
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

    pub(crate) fn offset_to_line_col(&self, text: &Rope, offset: usize) -> (usize, usize) {
        let line = self.line_of_offset(text, offset);
        (line, offset - self.offset_of_line(text, line))
    }

    pub(crate) fn line_col_to_offset(&self, text: &Rope, line: usize, col: usize) -> usize {
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

    pub(crate) fn rewrap(&mut self, text: &Rope, wrap_col: usize) {
        if wrap_col > 0 {
            self.breaks = Some(linewrap::linewrap(text, wrap_col));
            self.wrap_col = WrapWidth::Bytes(wrap_col);
        } else {
            self.breaks = None
        }
    }

    /// Generate line breaks based on width measurement. Currently batch-mode,
    /// and currently in a debugging state.
    pub(crate) fn wrap_width(&mut self, text: &Rope, width_cache: &mut WidthCache,
                             client: &Client, style_spans: &Spans<Style>)
    {
        let _t = trace_block("View::wrap_width", &["core"]);
        self.breaks = Some(linewrap::linewrap_width(text, width_cache,
                                                    style_spans, client,
                                                    self.size.width));
        self.wrap_col = WrapWidth::Width(self.size.width);
    }

    /// Updates the view after the text has been modified by the given `delta`.
    /// This method is responsible for updating the cursors, and also for
    /// recomputing line wraps.
    pub fn after_edit(&mut self, text: &Rope, last_text: &Rope,
                      delta: &Delta<RopeInfo>, client: &Client,
                      width_cache: &mut WidthCache, keep_selections: bool)
    {
        let (iv, new_len) = delta.summary();
        if let Some(breaks) = self.breaks.as_mut() {
            match self.wrap_col {
                WrapWidth::None => (),
                WrapWidth::Bytes(col) => linewrap::rewrap(breaks, text, iv,
                                                          new_len, col),
                WrapWidth::Width(px) =>
                    linewrap::rewrap_width(breaks, text, width_cache,
                                           client, iv, new_len, px),
            }
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
        // Any edit cancels a drag. This is good behavior for edits initiated through
        // the front-end, but perhaps not for async edits.
        self.drag_state = None;

        // update only find highlights affected by change
        for find in &mut self.find {
            find.update_highlights(text, delta);
        }

        self.find_changed = FindStatusChange::Matches;

        // Note: for committing plugin edits, we probably want to know the priority
        // of the delta so we can set the cursor before or after the edit, as needed.
        let new_sel = self.selection.apply_delta(delta, true, keep_selections);
        self.set_selection_for_edit(text, new_sel);
    }

    fn do_selection_for_find(&mut self, text: &Rope, case_sensitive: bool) {
        // set last selection or word under current cursor as search query
        let search_query = match self.selection.last() {
            Some(region) => {
                if !region.is_caret() {
                    text.slice_to_string(region.min(), region.max())
                } else {
                    let (start, end) = {
                        let mut word_cursor = WordCursor::new(text, region.max());
                        word_cursor.select_word()
                    };
                    text.slice_to_string(start, end)
                }
            },
            _ => return
        };

        self.find_changed = FindStatusChange::All;
        self.set_dirty(text);

        // todo: this will be changed once multiple queries are supported
        // todo: for now only a single search query is supported however in the future
        // todo: the correct Find instance needs to be updated with the new parameters
        if self.find.is_empty() {
            self.find.push(Find::new());
        }

        self.find.first_mut().unwrap().do_find(text, search_query, case_sensitive, false, true);
    }

    pub fn do_find(&mut self, text: &Rope, chars: String, case_sensitive: bool, is_regex: bool,
                   whole_words: bool) {
        self.set_dirty(text);
        self.find_changed = FindStatusChange::Matches;

        // todo: this will be changed once multiple queries are supported
        // todo: for now only a single search query is supported however in the future
        // todo: the correct Find instance needs to be updated with the new parameters
        if self.find.is_empty() {
            self.find.push(Find::new());
        }

        self.find.first_mut().unwrap().do_find(text, chars, case_sensitive, is_regex, whole_words);
    }

    /// Selects the next find match.
    pub fn do_find_next(&mut self, text: &Rope, reverse: bool, wrap: bool, allow_same: bool,
                     modify_selection: &SelectionModifier) {
        self.select_next_occurrence(text, reverse, false, allow_same, modify_selection);
        if self.scroll_to.is_none() && wrap {
            self.select_next_occurrence(text, reverse, true, allow_same, modify_selection);
        }
    }

    /// Selects all find matches.
    pub fn do_find_all(&mut self, text: &Rope) {
        let mut selection = Selection::new();
        for find in self.find.iter() {
            for &occurrence in find.occurrences().iter() {
                selection.add_region(occurrence);
            }
        }

        if !selection.is_empty() { // todo: invalidate so that nothing selected accidentally replaced
            self.set_selection(text, selection);
        }
    }

    /// Select the next occurrence relative to the last cursor. `reverse` determines whether the
    /// next occurrence before (`true`) or after (`false`) the last cursor is selected. `wrapped`
    /// indicates a search for the next occurrence past the end of the file.
    pub fn select_next_occurrence(&mut self, text: &Rope, reverse: bool, wrapped: bool,
                                  _allow_same: bool, modify_selection: &SelectionModifier) {
        // multiple queries; select closest occurrence
        let closest_occurrence = self.find.iter().flat_map(|x|
            x.next_occurrence(text, reverse, wrapped, &self.selection)
        ).min_by_key(|x| {
            match reverse {
                true => x.end,
                false => x.start
            }
        });

        if let Some(occ) = closest_occurrence {
            match modify_selection {
                SelectionModifier::Set => self.set_selection(text, occ),
                SelectionModifier::Add => {
                    let mut selection = self.selection.clone();
                    selection.add_region(occ);
                    self.set_selection(text, selection);
                },
                SelectionModifier::AddRemovingCurrent => {
                    let mut selection = self.selection.clone();

                    if let Some(last_selection) = self.selection.last() {
                        if !last_selection.is_caret() {
                            selection.delete_range(last_selection.min(), last_selection.max(), false);
                        }
                    }

                    selection.add_region(occ);
                    self.set_selection(text, selection);
                }
                _ => { }
            }
        }
    }

    fn do_set_replace(&mut self, chars: String, preserve_case: bool) {
        self.replace = Some(Replace { chars, preserve_case });
        self.replace_changed = true;
    }

    fn do_selection_for_replace(&mut self, text: &Rope) {
        // set last selection or word under current cursor as replacement string
        let replacement = match self.selection.last() {
            Some(region) => {
                if !region.is_caret() {
                    text.slice_to_string(region.min(), region.max())
                } else {
                    let (start, end) = {
                        let mut word_cursor = WordCursor::new(text, region.max());
                        word_cursor.select_word()
                    };
                    text.slice_to_string(start, end)
                }
            },
            _ => return
        };

        self.set_dirty(text);
        self.do_set_replace(replacement, false);
    }

    /// Get the line range of a selected region.
    pub fn get_line_range(&self, text: &Rope, region: &SelRegion) -> Range<usize> {
        let (first_line, _) = self.offset_to_line_col(text, region.min());
        let (mut last_line, last_col) = self.offset_to_line_col(text, region.max());
        if last_col == 0 && last_line > first_line {
            last_line -= 1;
        }

        first_line..(last_line + 1)
    }

    pub fn get_caret_offset(&self) -> Option<usize> {
        match self.selection.len() {
            1 if self.selection[0].is_caret() => {
                let offset = self.selection[0].start;
                Some(offset)
            }
            _ => None
        }
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
