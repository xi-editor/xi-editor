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

use std::cell::RefCell;
use std::cmp::{max, min};
use std::iter;
use std::ops::Range;

use serde_json::Value;

use crate::annotations::{AnnotationStore, Annotations, ToAnnotation};
use crate::client::{Client, Update, UpdateOp};
use crate::edit_types::ViewEvent;
use crate::find::{Find, FindStatus};
use crate::line_cache_shadow::{self, LineCacheShadow, RenderPlan, RenderTactic};
use crate::line_offset::LineOffset;
use crate::linewrap::{InvalLines, Lines, VisualLine, WrapWidth};
use crate::movement::{region_movement, selection_movement, Movement};
use crate::plugins::PluginId;
use crate::rpc::{FindQuery, GestureType, MouseAction, SelectionGranularity, SelectionModifier};
use crate::selection::{Affinity, InsertDrift, SelRegion, Selection};
use crate::styles::{Style, ThemeStyleMap};
use crate::tabs::{BufferId, Counter, ViewId};
use crate::width_cache::WidthCache;
use crate::word_boundaries::WordCursor;
use xi_rope::spans::Spans;
use xi_rope::{Cursor, Interval, LinesMetric, Rope, RopeDelta};
use xi_trace::trace_block;

type StyleMap = RefCell<ThemeStyleMap>;

/// A flag used to indicate when legacy actions should modify selections
const FLAG_SELECT: u64 = 2;

/// Size of batches as number of bytes used during incremental find.
const FIND_BATCH_SIZE: usize = 500000;

/// A view to a buffer. It is the buffer plus additional information
/// like line breaks and selection state.
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
    lines: Lines,

    /// Front end's line cache state for this view. See the `LineCacheShadow`
    /// description for the invariant.
    lc_shadow: LineCacheShadow,

    /// New offset to be scrolled into position after an edit.
    scroll_to: Option<usize>,

    /// The state for finding text for this view.
    /// Each instance represents a separate search query.
    find: Vec<Find>,

    /// Tracks the IDs for additional search queries in find.
    find_id_counter: Counter,

    /// Tracks whether there has been changes in find results or find parameters.
    /// This is used to determined whether FindStatus should be sent to the frontend.
    find_changed: FindStatusChange,

    /// Tracks the progress of incremental find.
    find_progress: FindProgress,

    /// Tracks whether find highlights should be rendered.
    /// Highlights are only rendered when search dialog is open.
    highlight_find: bool,

    /// The state for replacing matches for this view.
    replace: Option<Replace>,

    /// Tracks whether the replacement string or replace parameters changed.
    replace_changed: bool,

    /// Annotations provided by plugins.
    annotations: AnnotationStore,
}

/// Indicates what changed in the find state.
#[derive(PartialEq, Debug)]
enum FindStatusChange {
    /// None of the find parameters or number of matches changed.
    None,

    /// Find parameters and number of matches changed.
    All,

    /// Only number of matches changed
    Matches,
}

/// Indicates what changed in the find state.
#[derive(PartialEq, Debug, Clone)]
enum FindProgress {
    /// Incremental find is done/not running.
    Ready,

    /// The find process just started.
    Started,

    /// Incremental find is in progress. Keeps tracked of already searched range.
    InProgress(Range<usize>),
}

/// Contains replacement string and replace options.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize, Clone)]
pub struct Replace {
    /// Replacement string.
    pub chars: String,
    pub preserve_case: bool,
}

/// A size, in pixel units (not display pixels).
#[derive(Debug, Default, PartialEq, Serialize, Deserialize, Clone)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

/// State required to resolve a drag gesture into a selection.
struct DragState {
    /// All the selection regions other than the one being dragged.
    base_sel: Selection,

    /// Start of the region selected when drag was started (region is
    /// assumed to be forward).
    min: usize,

    /// End of the region selected when drag was started.
    max: usize,

    granularity: SelectionGranularity,
}

impl View {
    pub fn new(view_id: ViewId, buffer_id: BufferId) -> View {
        View {
            view_id,
            buffer_id,
            pending_render: false,
            selection: SelRegion::caret(0).into(),
            scroll_to: Some(0),
            size: Size::default(),
            drag_state: None,
            first_line: 0,
            height: 10,
            lines: Lines::default(),
            lc_shadow: LineCacheShadow::default(),
            find: Vec::new(),
            find_id_counter: Counter::default(),
            find_changed: FindStatusChange::None,
            find_progress: FindProgress::Ready,
            highlight_find: false,
            replace: None,
            replace_changed: false,
            annotations: AnnotationStore::new(),
        }
    }

    pub(crate) fn get_buffer_id(&self) -> BufferId {
        self.buffer_id
    }

    pub(crate) fn get_view_id(&self) -> ViewId {
        self.view_id
    }

    pub(crate) fn get_lines(&self) -> &Lines {
        &self.lines
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

    pub(crate) fn update_wrap_settings(&mut self, text: &Rope, wrap_cols: usize, word_wrap: bool) {
        let wrap_width = match (word_wrap, wrap_cols) {
            (true, _) => WrapWidth::Width(self.size.width),
            (false, 0) => WrapWidth::None,
            (false, cols) => WrapWidth::Bytes(cols),
        };
        self.lines.set_wrap_width(text, wrap_width);
    }

    pub(crate) fn needs_more_wrap(&self) -> bool {
        !self.lines.is_converged()
    }

    pub(crate) fn needs_wrap_in_visible_region(&self, text: &Rope) -> bool {
        if self.lines.is_converged() {
            false
        } else {
            let visible_region = self.interval_of_visible_region(text);
            self.lines.interval_needs_wrap(visible_region)
        }
    }

    pub(crate) fn find_in_progress(&self) -> bool {
        matches!(self.find_progress, FindProgress::InProgress(_) | FindProgress::Started)
    }

    pub(crate) fn do_edit(&mut self, text: &Rope, cmd: ViewEvent) {
        use self::ViewEvent::*;
        match cmd {
            Move(movement) => self.do_move(text, movement, false),
            ModifySelection(movement) => self.do_move(text, movement, true),
            SelectAll => self.select_all(text),
            Scroll(range) => self.set_scroll(range.first, range.last),
            AddSelectionAbove => self.add_selection_by_movement(text, Movement::UpExactPosition),
            AddSelectionBelow => self.add_selection_by_movement(text, Movement::DownExactPosition),
            Gesture { line, col, ty } => self.do_gesture(text, line, col, ty),
            GotoLine { line } => self.goto_line(text, line),
            Find { chars, case_sensitive, regex, whole_words } => {
                let id = self.find.first().map(|q| q.id());
                let query_changes = FindQuery { id, chars, case_sensitive, regex, whole_words };
                self.set_find(text, [query_changes].to_vec())
            }
            MultiFind { queries } => self.set_find(text, queries),
            FindNext { wrap_around, allow_same, modify_selection } => {
                self.do_find_next(text, false, wrap_around, allow_same, &modify_selection)
            }
            FindPrevious { wrap_around, allow_same, modify_selection } => {
                self.do_find_next(text, true, wrap_around, allow_same, &modify_selection)
            }
            FindAll => self.do_find_all(text),
            Click(MouseAction { line, column, flags, click_count }) => {
                // Deprecated (kept for client compatibility):
                // should be removed in favor of do_gesture
                warn!("Usage of click is deprecated; use do_gesture");
                if (flags & FLAG_SELECT) != 0 {
                    self.do_gesture(
                        text,
                        line,
                        column,
                        GestureType::SelectExtend { granularity: SelectionGranularity::Point },
                    )
                } else if click_count == Some(2) {
                    self.do_gesture(text, line, column, GestureType::WordSelect)
                } else if click_count == Some(3) {
                    self.do_gesture(text, line, column, GestureType::LineSelect)
                } else {
                    self.do_gesture(text, line, column, GestureType::PointSelect)
                }
            }
            Drag(MouseAction { line, column, .. }) => {
                warn!("Usage of drag is deprecated; use gesture instead");
                self.do_gesture(text, line, column, GestureType::Drag)
            }
            CollapseSelections => self.collapse_selections(text),
            HighlightFind { visible } => {
                self.highlight_find = visible;
                self.find_changed = FindStatusChange::All;
                self.set_dirty(text);
            }
            SelectionForFind { case_sensitive } => self.do_selection_for_find(text, case_sensitive),
            Replace { chars, preserve_case } => self.do_set_replace(chars, preserve_case),
            SelectionForReplace => self.do_selection_for_replace(text),
            SelectionIntoLines => self.do_split_selection_into_lines(text),
        }
    }

    fn do_gesture(&mut self, text: &Rope, line: u64, col: u64, ty: GestureType) {
        let line = line as usize;
        let col = col as usize;
        let offset = self.line_col_to_offset(text, line, col);
        match ty {
            GestureType::Select { granularity, multi } => {
                self.select(text, offset, granularity, multi)
            }
            GestureType::SelectExtend { granularity } => {
                self.extend_selection(text, offset, granularity)
            }
            GestureType::Drag => self.do_drag(text, offset, Affinity::default()),

            _ => {
                warn!("Deprecated gesture type sent to do_gesture method");
            }
        }
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

    /// Removes any selection present at the given offset.
    /// Returns true if a selection was removed, false otherwise.
    pub fn deselect_at_offset(&mut self, text: &Rope, offset: usize) -> bool {
        if !self.selection.regions_in_range(offset, offset).is_empty() {
            let mut sel = self.selection.clone();
            sel.delete_range(offset, offset, true);
            if !sel.is_empty() {
                self.drag_state = None;
                self.set_selection_raw(text, sel);
                return true;
            }
        }
        false
    }

    /// Move the selection by the given movement. Return value is the offset of
    /// a point that should be scrolled into view.
    ///
    /// If `modify` is `true`, the selections are modified, otherwise the results
    /// of individual region movements become carets.
    pub fn do_move(&mut self, text: &Rope, movement: Movement, modify: bool) {
        self.drag_state = None;
        let new_sel =
            selection_movement(movement, &self.selection, self, self.scroll_height(), text, modify);
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
            let new_region =
                region_movement(movement, region, self, self.scroll_height(), text, false);
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

    pub fn update_annotations(
        &mut self,
        plugin: PluginId,
        interval: Interval,
        annotations: Annotations,
    ) {
        self.annotations.update(plugin, interval, annotations)
    }

    /// Select entire buffer.
    ///
    /// Note: unlike movement based selection, this does not scroll.
    pub fn select_all(&mut self, text: &Rope) {
        let selection = SelRegion::new(0, text.len()).into();
        self.set_selection_raw(text, selection);
    }

    /// Finds the unit of text containing the given offset.
    fn unit(&self, text: &Rope, offset: usize, granularity: SelectionGranularity) -> Interval {
        match granularity {
            SelectionGranularity::Point => Interval::new(offset, offset),
            SelectionGranularity::Word => {
                let mut word_cursor = WordCursor::new(text, offset);
                let (start, end) = word_cursor.select_word();
                Interval::new(start, end)
            }
            SelectionGranularity::Line => {
                let (line, _) = self.offset_to_line_col(text, offset);
                let (start, end) = self.lines.logical_line_range(text, line);
                Interval::new(start, end)
            }
        }
    }

    /// Selects text with a certain granularity and supports multi_selection
    fn select(
        &mut self,
        text: &Rope,
        offset: usize,
        granularity: SelectionGranularity,
        multi: bool,
    ) {
        // If multi-select is enabled, toggle existing regions
        if multi
            && granularity == SelectionGranularity::Point
            && self.deselect_at_offset(text, offset)
        {
            return;
        }

        let region = self.unit(text, offset, granularity).into();

        let base_sel = match multi {
            true => self.selection.clone(),
            false => Selection::new(),
        };
        let mut selection = base_sel.clone();
        selection.add_region(region);
        self.set_selection(text, selection);

        self.drag_state =
            Some(DragState { base_sel, min: region.start, max: region.end, granularity });
    }

    /// Extends an existing selection (eg. when the user performs SHIFT + click).
    pub fn extend_selection(
        &mut self,
        text: &Rope,
        offset: usize,
        granularity: SelectionGranularity,
    ) {
        if self.sel_regions().is_empty() {
            return;
        }

        let (base_sel, last) = {
            let mut base = Selection::new();
            let (last, rest) = self.sel_regions().split_last().unwrap();
            for &region in rest {
                base.add_region(region);
            }
            (base, *last)
        };

        let mut sel = base_sel.clone();
        self.drag_state =
            Some(DragState { base_sel, min: last.start, max: last.start, granularity });

        let start = (last.start, last.start);
        let new_region = self.range_region(text, start, offset, granularity);

        // TODO: small nit, merged region should be backward if end < start.
        // This could be done by explicitly overriding, or by tweaking the
        // merge logic.
        sel.add_region(new_region);
        self.set_selection(text, sel);
    }

    /// Splits current selections into lines.
    fn do_split_selection_into_lines(&mut self, text: &Rope) {
        let mut selection = Selection::new();

        for region in self.selection.iter() {
            if region.is_caret() {
                selection.add_region(SelRegion::caret(region.max()));
            } else {
                let mut cursor = Cursor::new(text, region.min());

                while cursor.pos() < region.max() {
                    let sel_start = cursor.pos();
                    let end_of_line = match cursor.next::<LinesMetric>() {
                        Some(end) if end >= region.max() => max(0, region.max() - 1),
                        Some(end) => max(0, end - 1),
                        None if cursor.pos() == text.len() => cursor.pos(),
                        _ => break,
                    };

                    selection.add_region(SelRegion::new(sel_start, end_of_line));
                }
            }
        }

        self.set_selection_raw(text, selection);
    }

    /// Does a drag gesture, setting the selection from a combination of the drag
    /// state and new offset.
    fn do_drag(&mut self, text: &Rope, offset: usize, affinity: Affinity) {
        let new_sel = self.drag_state.as_ref().map(|drag_state| {
            let mut sel = drag_state.base_sel.clone();
            let start = (drag_state.min, drag_state.max);
            let new_region = self.range_region(text, start, offset, drag_state.granularity);
            sel.add_region(new_region.with_horiz(None).with_affinity(affinity));
            sel
        });

        if let Some(sel) = new_sel {
            self.set_selection(text, sel);
        }
    }

    /// Creates a `SelRegion` for range select or drag operations.
    pub fn range_region(
        &self,
        text: &Rope,
        start: (usize, usize),
        offset: usize,
        granularity: SelectionGranularity,
    ) -> SelRegion {
        let (min_start, max_start) = start;
        let end = self.unit(text, offset, granularity);
        let (min_end, max_end) = (end.start, end.end);
        if offset >= min_start {
            SelRegion::new(min_start, max_end)
        } else {
            SelRegion::new(max_start, min_end)
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

    // Encode a single line with its styles and cursors in JSON.
    // If "text" is not specified, don't add "text" to the output.
    // If "style_spans" are not specified, don't add "styles" to the output.
    fn encode_line(
        &self,
        client: &Client,
        styles: &StyleMap,
        line: VisualLine,
        text: Option<&Rope>,
        style_spans: Option<&Spans<Style>>,
        last_pos: usize,
    ) -> Value {
        let start_pos = line.interval.start;
        let pos = line.interval.end;
        let mut cursors = Vec::new();
        let mut selections = Vec::new();
        for region in self.selection.regions_in_range(start_pos, pos) {
            // cursor
            let c = region.end;

            if (c > start_pos && c < pos)
                || (!region.is_upstream() && c == start_pos)
                || (region.is_upstream() && c == pos)
                || (c == pos && c == last_pos)
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
            for find in &self.find {
                let mut cur_hls = Vec::new();
                for region in find.occurrences().regions_in_range(start_pos, pos) {
                    let sel_start_ix = clamp(region.min(), start_pos, pos) - start_pos;
                    let sel_end_ix = clamp(region.max(), start_pos, pos) - start_pos;
                    if sel_end_ix > sel_start_ix {
                        cur_hls.push((sel_start_ix, sel_end_ix));
                    }
                }
                hls.push(cur_hls);
            }
        }

        let mut result = json!({});

        if let Some(text) = text {
            result["text"] = json!(text.slice_to_cow(start_pos..pos));
        }
        if let Some(style_spans) = style_spans {
            result["styles"] = json!(self.encode_styles(
                client,
                styles,
                start_pos,
                pos,
                &selections,
                &hls,
                style_spans
            ));
        }
        if !cursors.is_empty() {
            result["cursor"] = json!(cursors);
        }
        if let Some(line_num) = line.line_num {
            result["ln"] = json!(line_num);
        }
        result
    }

    pub fn encode_styles(
        &self,
        client: &Client,
        styles: &StyleMap,
        start: usize,
        end: usize,
        sel: &[(usize, usize)],
        hls: &Vec<Vec<(usize, usize)>>,
        style_spans: &Spans<Style>,
    ) -> Vec<isize> {
        let mut encoded_styles = Vec::new();
        assert!(start <= end, "{} {}", start, end);
        let style_spans = style_spans.subseq(Interval::new(start, end));

        let mut ix = 0;
        // we add the special find highlights (1 to N) and selection (0) styles first.
        // We add selection after find because we want it to be preferred if the
        // same span exists in both sets (as when there is an active selection)
        for (index, cur_find_hls) in hls.iter().enumerate() {
            for &(sel_start, sel_end) in cur_find_hls {
                encoded_styles.push((sel_start as isize) - ix);
                encoded_styles.push(sel_end as isize - sel_start as isize);
                encoded_styles.push(index as isize + 1);
                ix = sel_end as isize;
            }
        }
        for &(sel_start, sel_end) in sel {
            encoded_styles.push((sel_start as isize) - ix);
            encoded_styles.push(sel_end as isize - sel_start as isize);
            encoded_styles.push(0);
            ix = sel_end as isize;
        }
        for (iv, style) in style_spans.iter() {
            let style_id = self.get_or_def_style_id(client, styles, style);
            encoded_styles.push((iv.start() as isize) - ix);
            encoded_styles.push(iv.end() as isize - iv.start() as isize);
            encoded_styles.push(style_id as isize);
            ix = iv.end() as isize;
        }
        encoded_styles
    }

    fn get_or_def_style_id(&self, client: &Client, style_map: &StyleMap, style: &Style) -> usize {
        let mut style_map = style_map.borrow_mut();
        if let Some(ix) = style_map.lookup(style) {
            return ix;
        }
        let ix = style_map.add(style);
        let style = style_map.merge_with_default(style);
        client.def_style(&style.to_json(ix));
        ix
    }

    fn send_update_for_plan(
        &mut self,
        text: &Rope,
        client: &Client,
        styles: &StyleMap,
        style_spans: &Spans<Style>,
        plan: &RenderPlan,
        pristine: bool,
    ) {
        // every time current visible range changes, annotations are sent to frontend
        let start_off = self.offset_of_line(text, self.first_line);
        let end_off = self.offset_of_line(text, self.first_line + self.height + 2);
        let visible_range = Interval::new(start_off, end_off);
        let selection_annotations =
            self.selection.get_annotations(visible_range, self, text).to_json();
        let find_annotations =
            self.find.iter().map(|f| f.get_annotations(visible_range, self, text).to_json());
        let plugin_annotations =
            self.annotations.iter_range(self, text, visible_range).map(|a| a.to_json());

        let annotations = iter::once(selection_annotations)
            .chain(find_annotations)
            .chain(plugin_annotations)
            .collect::<Vec<_>>();

        if !self.lc_shadow.needs_render(plan) {
            let total_lines = self.line_of_offset(text, text.len()) + 1;
            let update =
                Update { ops: vec![UpdateOp::copy(total_lines, 1)], pristine, annotations };
            client.update_view(self.view_id, &update);
            return;
        }

        // send updated find status only if there have been changes
        if self.find_changed != FindStatusChange::None {
            let matches_only = self.find_changed == FindStatusChange::Matches;
            client.find_status(self.view_id, &json!(self.find_status(text, matches_only)));
            self.find_changed = FindStatusChange::None;
        }

        // send updated replace status if changed
        if self.replace_changed {
            if let Some(replace) = self.get_replace() {
                client.replace_status(self.view_id, &json!(replace))
            }
        }

        let mut b = line_cache_shadow::Builder::new();
        let mut ops = Vec::new();
        let mut line_num = 0; // tracks old line cache

        for seg in self.lc_shadow.iter_with_plan(plan) {
            match seg.tactic {
                RenderTactic::Discard => {
                    ops.push(UpdateOp::invalidate(seg.n));
                    b.add_span(seg.n, 0, 0);
                }
                RenderTactic::Preserve | RenderTactic::Render => {
                    // Depending on the state of TEXT_VALID, STYLES_VALID and
                    // CURSOR_VALID, perform one of the following actions:
                    //
                    //   - All the three are valid => send the "copy" op
                    //     (+leading "skip" to catch up with "ln" to update);
                    //
                    //   - Text and styles are valid, cursors are not => same,
                    //     but send an "update" op instead of "copy" to move
                    //     the cursors;
                    //
                    //   - Text or styles are invalid:
                    //     => send "invalidate" if RenderTactic is "Preserve";
                    //     => send "skip"+"insert" (recreate the lines) if
                    //        RenderTactic is "Render".
                    if (seg.validity & line_cache_shadow::TEXT_VALID) != 0
                        && (seg.validity & line_cache_shadow::STYLES_VALID) != 0
                    {
                        let n_skip = seg.their_line_num - line_num;
                        if n_skip > 0 {
                            ops.push(UpdateOp::skip(n_skip));
                        }
                        let line_offset = self.offset_of_line(text, seg.our_line_num);
                        let logical_line = text.line_of_offset(line_offset);
                        if (seg.validity & line_cache_shadow::CURSOR_VALID) != 0 {
                            // ALL_VALID; copy lines as-is
                            ops.push(UpdateOp::copy(seg.n, logical_line + 1));
                        } else {
                            // !CURSOR_VALID; update cursors
                            let start_line = seg.our_line_num;

                            let encoded_lines = self
                                .lines
                                .iter_lines(text, start_line)
                                .take(seg.n)
                                .map(|l| {
                                    self.encode_line(
                                        client,
                                        styles,
                                        l,
                                        /* text = */ None,
                                        /* style_spans = */ None,
                                        text.len(),
                                    )
                                })
                                .collect::<Vec<_>>();

                            let logical_line_opt =
                                if logical_line == 0 { None } else { Some(logical_line + 1) };
                            ops.push(UpdateOp::update(encoded_lines, logical_line_opt));
                        }
                        b.add_span(seg.n, seg.our_line_num, seg.validity);
                        line_num = seg.their_line_num + seg.n;
                    } else if seg.tactic == RenderTactic::Preserve {
                        ops.push(UpdateOp::invalidate(seg.n));
                        b.add_span(seg.n, 0, 0);
                    } else if seg.tactic == RenderTactic::Render {
                        let start_line = seg.our_line_num;
                        let encoded_lines = self
                            .lines
                            .iter_lines(text, start_line)
                            .take(seg.n)
                            .map(|l| {
                                self.encode_line(
                                    client,
                                    styles,
                                    l,
                                    Some(text),
                                    Some(style_spans),
                                    text.len(),
                                )
                            })
                            .collect::<Vec<_>>();
                        debug_assert_eq!(encoded_lines.len(), seg.n);
                        ops.push(UpdateOp::insert(encoded_lines));
                        b.add_span(seg.n, seg.our_line_num, line_cache_shadow::ALL_VALID);
                    }
                }
            }
        }

        self.lc_shadow = b.build();
        for find in &mut self.find {
            find.set_hls_dirty(false)
        }

        let update = Update { ops, pristine, annotations };
        client.update_view(self.view_id, &update);
    }

    /// Determines the current number of find results and search parameters to send them to
    /// the frontend.
    pub fn find_status(&self, text: &Rope, matches_only: bool) -> Vec<FindStatus> {
        self.find
            .iter()
            .map(|find| find.find_status(self, text, matches_only))
            .collect::<Vec<FindStatus>>()
    }

    /// Update front-end with any changes to view since the last time sent.
    /// The `pristine` argument indicates whether or not the buffer has
    /// unsaved changes.
    pub fn render_if_dirty(
        &mut self,
        text: &Rope,
        client: &Client,
        styles: &StyleMap,
        style_spans: &Spans<Style>,
        pristine: bool,
    ) {
        let height = self.line_of_offset(text, text.len()) + 1;
        let plan = RenderPlan::create(height, self.first_line, self.height);
        self.send_update_for_plan(text, client, styles, style_spans, &plan, pristine);
        if let Some(new_scroll_pos) = self.scroll_to.take() {
            let (line, col) = self.offset_to_line_col(text, new_scroll_pos);
            client.scroll_to(self.view_id, line, col);
        }
    }

    // Send the requested lines even if they're outside the current scroll region.
    pub fn request_lines(
        &mut self,
        text: &Rope,
        client: &Client,
        styles: &StyleMap,
        style_spans: &Spans<Style>,
        first_line: usize,
        last_line: usize,
        pristine: bool,
    ) {
        let height = self.line_of_offset(text, text.len()) + 1;
        let mut plan = RenderPlan::create(height, self.first_line, self.height);
        plan.request_lines(first_line, last_line);
        self.send_update_for_plan(text, client, styles, style_spans, &plan, pristine);
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

    /// Returns the byte range of the currently visible lines.
    fn interval_of_visible_region(&self, text: &Rope) -> Interval {
        let start = self.offset_of_line(text, self.first_line);
        let end = self.offset_of_line(text, self.first_line + self.height + 1);
        Interval::new(start, end)
    }

    /// Generate line breaks, based on current settings. Currently batch-mode,
    /// and currently in a debugging state.
    pub(crate) fn rewrap(
        &mut self,
        text: &Rope,
        width_cache: &mut WidthCache,
        client: &Client,
        spans: &Spans<Style>,
    ) {
        let _t = trace_block("View::rewrap", &["core"]);
        let visible = self.first_line..self.first_line + self.height;
        let inval = self.lines.rewrap_chunk(text, width_cache, client, spans, visible);
        if let Some(InvalLines { start_line, inval_count, new_count }) = inval {
            self.lc_shadow.edit(start_line, start_line + inval_count, new_count);
        }
    }

    /// Updates the view after the text has been modified by the given `delta`.
    /// This method is responsible for updating the cursors, and also for
    /// recomputing line wraps.
    pub fn after_edit(
        &mut self,
        text: &Rope,
        last_text: &Rope,
        delta: &RopeDelta,
        client: &Client,
        width_cache: &mut WidthCache,
        drift: InsertDrift,
    ) {
        let visible = self.first_line..self.first_line + self.height;
        match self.lines.after_edit(text, last_text, delta, width_cache, client, visible) {
            Some(InvalLines { start_line, inval_count, new_count }) => {
                self.lc_shadow.edit(start_line, start_line + inval_count, new_count);
            }
            None => self.set_dirty(text),
        }

        // Any edit cancels a drag. This is good behavior for edits initiated through
        // the front-end, but perhaps not for async edits.
        self.drag_state = None;

        // all annotations that come after the edit need to be invalidated
        let (iv, _) = delta.summary();
        self.annotations.invalidate(iv);

        // update only find highlights affected by change
        for find in &mut self.find {
            find.update_highlights(text, delta);
            self.find_changed = FindStatusChange::All;
        }

        // Note: for committing plugin edits, we probably want to know the priority
        // of the delta so we can set the cursor before or after the edit, as needed.
        let new_sel = self.selection.apply_delta(delta, true, drift);
        self.set_selection_for_edit(text, new_sel);
    }

    fn do_selection_for_find(&mut self, text: &Rope, case_sensitive: bool) {
        // set last selection or word under current cursor as search query
        let search_query = match self.selection.last() {
            Some(region) => {
                if !region.is_caret() {
                    text.slice_to_cow(region)
                } else {
                    let (start, end) = {
                        let mut word_cursor = WordCursor::new(text, region.max());
                        word_cursor.select_word()
                    };
                    text.slice_to_cow(start..end)
                }
            }
            _ => return,
        };

        self.set_dirty(text);

        // set selection as search query for first find if no additional search queries are used
        // otherwise add new find with selection as search query
        if self.find.len() != 1 {
            self.add_find();
        }

        self.find.last_mut().unwrap().set_find(&search_query, case_sensitive, false, true);
        self.find_progress = FindProgress::Started;
    }

    fn add_find(&mut self) {
        let id = self.find_id_counter.next();
        self.find.push(Find::new(id));
    }

    fn set_find(&mut self, text: &Rope, queries: Vec<FindQuery>) {
        // checks if at least query has been changed, otherwise we don't need to rerun find
        let mut find_changed = queries.len() != self.find.len();

        // remove deleted queries
        self.find.retain(|f| queries.iter().any(|q| q.id == Some(f.id())));

        for query in &queries {
            let pos = match query.id {
                Some(id) => {
                    // update existing query
                    match self.find.iter().position(|f| f.id() == id) {
                        Some(p) => p,
                        None => return,
                    }
                }
                None => {
                    // add new query
                    self.add_find();
                    self.find.len() - 1
                }
            };

            if self.find[pos].set_find(
                &query.chars.clone(),
                query.case_sensitive,
                query.regex,
                query.whole_words,
            ) {
                find_changed = true;
            }
        }

        if find_changed {
            self.set_dirty(text);
            self.find_progress = FindProgress::Started;
        }
    }

    pub fn do_find(&mut self, text: &Rope) {
        let search_range = match &self.find_progress.clone() {
            FindProgress::Started => {
                // start incremental find on visible region
                let start = self.offset_of_line(text, self.first_line);
                let end = min(text.len(), start + FIND_BATCH_SIZE);
                self.find_changed = FindStatusChange::Matches;
                self.find_progress = FindProgress::InProgress(Range { start, end });
                Some((start, end))
            }
            FindProgress::InProgress(searched_range) => {
                if searched_range.start == 0 && searched_range.end >= text.len() {
                    // the entire text has been searched
                    // end find by executing multi-line regex queries on entire text
                    // stop incremental find
                    self.find_progress = FindProgress::Ready;
                    self.find_changed = FindStatusChange::All;
                    Some((0, text.len()))
                } else {
                    self.find_changed = FindStatusChange::Matches;
                    // expand find to un-searched regions
                    let start_off = self.offset_of_line(text, self.first_line);

                    // If there is unsearched text before the visible region, we want to include it in this search operation
                    let search_preceding_range = start_off.saturating_sub(searched_range.start)
                        < searched_range.end.saturating_sub(start_off)
                        && searched_range.start > 0;

                    if search_preceding_range || searched_range.end >= text.len() {
                        let start = searched_range.start.saturating_sub(FIND_BATCH_SIZE);
                        self.find_progress =
                            FindProgress::InProgress(Range { start, end: searched_range.end });
                        Some((start, searched_range.start))
                    } else if searched_range.end < text.len() {
                        let end = min(text.len(), searched_range.end + FIND_BATCH_SIZE);
                        self.find_progress =
                            FindProgress::InProgress(Range { start: searched_range.start, end });
                        Some((searched_range.end, end))
                    } else {
                        self.find_changed = FindStatusChange::All;
                        None
                    }
                }
            }
            _ => {
                self.find_changed = FindStatusChange::None;
                None
            }
        };

        if let Some((search_range_start, search_range_end)) = search_range {
            for query in &mut self.find {
                if !query.is_multiline_regex() {
                    query.update_find(text, search_range_start, search_range_end, true);
                } else {
                    // only execute multi-line regex queries if we are searching the entire text (last step)
                    if search_range_start == 0 && search_range_end == text.len() {
                        query.update_find(text, search_range_start, search_range_end, true);
                    }
                }
            }
        }
    }

    /// Selects the next find match.
    pub fn do_find_next(
        &mut self,
        text: &Rope,
        reverse: bool,
        wrap: bool,
        allow_same: bool,
        modify_selection: &SelectionModifier,
    ) {
        self.select_next_occurrence(text, reverse, false, allow_same, modify_selection);
        if self.scroll_to.is_none() && wrap {
            self.select_next_occurrence(text, reverse, true, allow_same, modify_selection);
        }
    }

    /// Selects all find matches.
    pub fn do_find_all(&mut self, text: &Rope) {
        let mut selection = Selection::new();
        for find in &self.find {
            for &occurrence in find.occurrences().iter() {
                selection.add_region(occurrence);
            }
        }

        if !selection.is_empty() {
            // todo: invalidate so that nothing selected accidentally replaced
            self.set_selection(text, selection);
        }
    }

    /// Select the next occurrence relative to the last cursor. `reverse` determines whether the
    /// next occurrence before (`true`) or after (`false`) the last cursor is selected. `wrapped`
    /// indicates a search for the next occurrence past the end of the file.
    pub fn select_next_occurrence(
        &mut self,
        text: &Rope,
        reverse: bool,
        wrapped: bool,
        _allow_same: bool,
        modify_selection: &SelectionModifier,
    ) {
        let (cur_start, cur_end) = match self.selection.last() {
            Some(sel) => (sel.min(), sel.max()),
            _ => (0, 0),
        };

        // multiple queries; select closest occurrence
        let closest_occurrence = self
            .find
            .iter()
            .flat_map(|x| x.next_occurrence(text, reverse, wrapped, &self.selection))
            .min_by_key(|x| match reverse {
                true if x.end > cur_end => 2 * text.len() - x.end,
                true => cur_end - x.end,
                false if x.start < cur_start => x.start + text.len(),
                false => x.start - cur_start,
            });

        if let Some(occ) = closest_occurrence {
            match modify_selection {
                SelectionModifier::Set => self.set_selection(text, occ),
                SelectionModifier::Add => {
                    let mut selection = self.selection.clone();
                    selection.add_region(occ);
                    self.set_selection(text, selection);
                }
                SelectionModifier::AddRemovingCurrent => {
                    let mut selection = self.selection.clone();

                    if let Some(last_selection) = self.selection.last() {
                        if !last_selection.is_caret() {
                            selection.delete_range(
                                last_selection.min(),
                                last_selection.max(),
                                false,
                            );
                        }
                    }

                    selection.add_region(occ);
                    self.set_selection(text, selection);
                }
                _ => {}
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
                    text.slice_to_cow(region)
                } else {
                    let (start, end) = {
                        let mut word_cursor = WordCursor::new(text, region.max());
                        word_cursor.select_word()
                    };
                    text.slice_to_cow(start..end)
                }
            }
            _ => return,
        };

        self.set_dirty(text);
        self.do_set_replace(replacement.into_owned(), false);
    }

    pub fn get_caret_offset(&self) -> Option<usize> {
        match self.selection.len() {
            1 if self.selection[0].is_caret() => {
                let offset = self.selection[0].start;
                Some(offset)
            }
            _ => None,
        }
    }
}

impl View {
    /// Exposed for benchmarking
    #[doc(hidden)]
    pub fn debug_force_rewrap_cols(&mut self, text: &Rope, cols: usize) {
        use xi_rpc::test_utils::DummyPeer;

        let spans: Spans<Style> = Spans::default();
        let mut width_cache = WidthCache::new();
        let client = Client::new(Box::new(DummyPeer));
        self.update_wrap_settings(text, cols, false);
        self.rewrap(text, &mut width_cache, &client, &spans);
    }
}

impl LineOffset for View {
    fn offset_of_line(&self, text: &Rope, line: usize) -> usize {
        self.lines.offset_of_visual_line(text, line)
    }

    fn line_of_offset(&self, text: &Rope, offset: usize) -> usize {
        self.lines.visual_line_of_offset(text, offset)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::FindQuery;

    #[test]
    fn incremental_find_update() {
        let mut view = View::new(1.into(), BufferId::new(2));
        let mut s = String::new();
        for _ in 0..(FIND_BATCH_SIZE - 2) {
            s += "x";
        }
        s += "aaaaaa";
        for _ in 0..(FIND_BATCH_SIZE) {
            s += "x";
        }
        s += "aaaaaa";
        assert_eq!(view.find_in_progress(), false);

        let text = Rope::from(&s);
        view.do_edit(
            &text,
            ViewEvent::Find {
                chars: "aaaaaa".to_string(),
                case_sensitive: false,
                regex: false,
                whole_words: false,
            },
        );
        view.do_find(&text);
        assert_eq!(view.find_in_progress(), true);
        view.do_find_all(&text);
        assert_eq!(view.sel_regions().len(), 1);
        assert_eq!(
            view.sel_regions().first(),
            Some(&SelRegion::new(FIND_BATCH_SIZE - 2, FIND_BATCH_SIZE + 6 - 2))
        );
        view.do_find(&text);
        assert_eq!(view.find_in_progress(), true);
        view.do_find_all(&text);
        assert_eq!(view.sel_regions().len(), 2);
    }

    #[test]
    fn incremental_find_codepoint_boundary() {
        let mut view = View::new(1.into(), BufferId::new(2));
        let mut s = String::new();
        for _ in 0..(FIND_BATCH_SIZE + 2) {
            s += "£€äßß";
        }

        assert_eq!(view.find_in_progress(), false);

        let text = Rope::from(&s);
        view.do_edit(
            &text,
            ViewEvent::Find {
                chars: "a".to_string(),
                case_sensitive: false,
                regex: false,
                whole_words: false,
            },
        );
        view.do_find(&text);
        assert_eq!(view.find_in_progress(), true);
        view.do_find_all(&text);
        assert_eq!(view.sel_regions().len(), 1); // cursor
    }

    #[test]
    fn selection_for_find() {
        let mut view = View::new(1.into(), BufferId::new(2));
        let text = Rope::from("hello hello world\n");
        view.set_selection(&text, SelRegion::new(6, 11));
        view.do_edit(&text, ViewEvent::SelectionForFind { case_sensitive: false });
        view.do_find(&text);
        view.do_find_all(&text);
        assert_eq!(view.sel_regions().len(), 2);
    }

    #[test]
    fn find_next() {
        let mut view = View::new(1.into(), BufferId::new(2));
        let text = Rope::from("hello hello world\n");
        view.do_edit(
            &text,
            ViewEvent::Find {
                chars: "foo".to_string(),
                case_sensitive: false,
                regex: false,
                whole_words: false,
            },
        );
        view.do_find(&text);
        view.do_find_next(&text, false, true, false, &SelectionModifier::Set);
        assert_eq!(view.sel_regions().len(), 1);
        assert_eq!(view.sel_regions().first(), Some(&SelRegion::new(0, 0))); // caret

        view.do_edit(
            &text,
            ViewEvent::Find {
                chars: "hello".to_string(),
                case_sensitive: false,
                regex: false,
                whole_words: false,
            },
        );
        view.do_find(&text);
        assert_eq!(view.sel_regions().len(), 1);
        view.do_find_next(&text, false, true, false, &SelectionModifier::Set);
        assert_eq!(view.sel_regions().first(), Some(&SelRegion::new(0, 5)));
        view.do_find_next(&text, false, true, false, &SelectionModifier::Set);
        assert_eq!(view.sel_regions().first(), Some(&SelRegion::new(6, 11)));
        view.do_find_next(&text, false, true, false, &SelectionModifier::Set);
        assert_eq!(view.sel_regions().first(), Some(&SelRegion::new(0, 5)));
        view.do_find_next(&text, true, true, false, &SelectionModifier::Set);
        assert_eq!(view.sel_regions().first(), Some(&SelRegion::new(6, 11)));

        view.do_find_next(&text, true, true, false, &SelectionModifier::Add);
        assert_eq!(view.sel_regions().len(), 2);
        view.do_find_next(&text, true, true, false, &SelectionModifier::AddRemovingCurrent);
        assert_eq!(view.sel_regions().len(), 1);
        view.do_find_next(&text, true, true, false, &SelectionModifier::None);
        assert_eq!(view.sel_regions().len(), 1);
    }

    #[test]
    fn find_all() {
        let mut view = View::new(1.into(), BufferId::new(2));
        let text = Rope::from("hello hello world\n hello!");
        view.do_edit(
            &text,
            ViewEvent::Find {
                chars: "foo".to_string(),
                case_sensitive: false,
                regex: false,
                whole_words: false,
            },
        );
        view.do_find(&text);
        view.do_find_all(&text);
        assert_eq!(view.sel_regions().len(), 1); // caret

        view.do_edit(
            &text,
            ViewEvent::Find {
                chars: "hello".to_string(),
                case_sensitive: false,
                regex: false,
                whole_words: false,
            },
        );
        view.do_find(&text);
        view.do_find_all(&text);
        assert_eq!(view.sel_regions().len(), 3);

        view.do_edit(
            &text,
            ViewEvent::Find {
                chars: "foo".to_string(),
                case_sensitive: false,
                regex: false,
                whole_words: false,
            },
        );
        view.do_find(&text);
        view.do_find_all(&text);
        assert_eq!(view.sel_regions().len(), 3);
    }

    #[test]
    fn multi_queries_find_next() {
        let mut view = View::new(1.into(), BufferId::new(2));
        let text = Rope::from("hello hello world\n hello!");
        let query1 = FindQuery {
            id: None,
            chars: "hello".to_string(),
            case_sensitive: false,
            regex: false,
            whole_words: false,
        };
        let query2 = FindQuery {
            id: None,
            chars: "o world".to_string(),
            case_sensitive: false,
            regex: false,
            whole_words: false,
        };
        view.do_edit(&text, ViewEvent::MultiFind { queries: vec![query1, query2] });
        view.do_find(&text);
        view.do_find_next(&text, false, true, false, &SelectionModifier::Set);
        assert_eq!(view.sel_regions().first(), Some(&SelRegion::new(0, 5)));
        view.do_find_next(&text, false, true, false, &SelectionModifier::Set);
        assert_eq!(view.sel_regions().first(), Some(&SelRegion::new(6, 11)));
        view.do_find_next(&text, false, true, false, &SelectionModifier::Set);
        assert_eq!(view.sel_regions().first(), Some(&SelRegion::new(10, 17)));
    }

    #[test]
    fn multi_queries_find_all() {
        let mut view = View::new(1.into(), BufferId::new(2));
        let text = Rope::from("hello hello world\n hello!");
        let query1 = FindQuery {
            id: None,
            chars: "hello".to_string(),
            case_sensitive: false,
            regex: false,
            whole_words: false,
        };
        let query2 = FindQuery {
            id: None,
            chars: "world".to_string(),
            case_sensitive: false,
            regex: false,
            whole_words: false,
        };
        view.do_edit(&text, ViewEvent::MultiFind { queries: vec![query1, query2] });
        view.do_find(&text);
        view.do_find_all(&text);
        assert_eq!(view.sel_regions().len(), 4);
    }
}
