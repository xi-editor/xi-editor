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
use std::io::Write;

use serde_json::value::Value;

use xi_rope::rope::{Rope, LinesMetric, RopeInfo};
use xi_rope::delta::{Delta};
use xi_rope::tree::Cursor;
use xi_rope::breaks::{Breaks, BreaksInfo, BreaksMetric, BreaksBaseMetric};
use xi_rope::interval::Interval;
use xi_rope::spans::Spans;

use tabs::{ViewIdentifier, TabCtx};
use styles;
use index_set::IndexSet;
use selection::{Affinity, Selection, SelRegion};

use linewrap;

const SCROLL_SLOP: usize = 2;

#[derive(Default, Clone)]
pub struct Style {
    pub fg: u32,
    pub font_style: u8,  // same as syntect, 1 = bold, 2 = underline, 4 = italic
}

pub struct View {
    pub view_id: ViewIdentifier,

    // The following 3 fields are the old (single selection) cursor state.
    // They will go away soon, but are kept for a gradual transition to the
    // new multi-select version.
    pub sel_start: usize,
    pub sel_end: usize,
    cursor_col: usize,

    /// The selection state for this view.
    selection: Selection,

    first_line: usize,  // vertical scroll position
    height: usize,  // height of visible portion
    breaks: Option<Breaks>,
    wrap_col: usize,

    // Ranges of lines held by the line cache in the front-end that are considered
    // valid.
    // TODO: separate tracking of text, cursors, and styles
    valid_lines: IndexSet,

    // The old selection (single cursor) selection state was updated.
    old_sel_dirty: bool,
    // The selection state was updated.
    sel_dirty: bool,

    // TODO: much finer grained tracking of dirty state
    dirty: bool,

    /// Tracks whether or not the view has unsaved changes.
    pristine: bool,
}

impl View {
    pub fn new<S: AsRef<str>>(view_id: S) -> View {
        View {
            view_id: view_id.as_ref().to_owned(),
            sel_start: 0,
            sel_end: 0,
            // used to maintain preferred hpos during vertical movement
            cursor_col: 0,
            selection: Selection::default(),
            first_line: 0,
            height: 10,
            breaks: None,
            wrap_col: 0,
            valid_lines: IndexSet::new(),
            old_sel_dirty: true,
            sel_dirty: true,
            dirty: true,
            pristine: true,
        }
    }

    pub fn set_scroll(&mut self, first: usize, last: usize) {
        self.first_line = first;
        self.height = last - first;
    }

    pub fn scroll_height(&self) -> usize {
        self.height
    }

    pub fn sel_min(&self) -> usize {
        min(self.sel_start, self.sel_end)
    }

    pub fn sel_max(&self) -> usize {
        max(self.sel_start, self.sel_end)
    }

    pub fn scroll_to_cursor(&mut self, text: &Rope) {
        let (line, _) = self.offset_to_line_col(text, self.sel_end);
        if line < self.first_line {
            self.first_line = line;
        } else if self.first_line + self.height <= line {
            self.first_line = line - (self.height - 1);
        }
    }

    pub fn set_cursor_col(&mut self, col: usize) {
        self.cursor_col = col;
    }

    pub fn get_cursor_col(&mut self) -> usize {
        self.cursor_col
    }

    // Render a single line, and advance cursors to next line.
    fn render_line<W: Write>(&self, tab_ctx: &TabCtx<W>, text: &Rope,
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

        let styles = self.render_styles(tab_ctx, start_pos, pos, &selections, style_spans);

        let mut result = json!({
            "text": &l_str,
            "styles": styles,
        });

        if !cursors.is_empty() {
            result["cursor"] = json!(cursors);
        }
        result
    }

    pub fn render_styles<W: Write>(&self, tab_ctx: &TabCtx<W>, start: usize, end: usize,
        sel: &[(usize, usize)], style_spans: &Spans<Style>) -> Vec<isize>
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
        for (iv, style) in style_spans.iter() {
            // This conversion will move because we'll store style id's in the spans
            // data structure. But we're changing things one piece at a time.
            let new_style = styles::Style {
                fg: style.fg,
                bg: 0,
                weight: if (style.font_style & 1) != 0 { 700 } else { 400},
                underline: (style.font_style & 2) != 0,
                italic: (style.font_style & 4) != 0,
            };
            let style_id = tab_ctx.get_style_id(&new_style);
            rendered_styles.push((iv.start() as isize) - ix);
            rendered_styles.push(iv.end() as isize - iv.start() as isize);
            rendered_styles.push(style_id as isize);
            ix = iv.end() as isize;
        }
        rendered_styles
    }

    pub fn send_update<W: Write>(&mut self, text: &Rope, tab_ctx: &TabCtx<W>, style_spans: &Spans<Style>,
        first_line: usize, last_line: usize)
    {
        let height = self.offset_to_line_col(text, text.len()).0 + 1;
        let last_line = min(last_line, height);
        let mut ops = Vec::new();
        if first_line > 0 {
            let op = if self.dirty { "invalidate" } else { "copy" };
            ops.push(self.build_update_op(op, None, first_line));
        }
        let first_line_offset = self.offset_of_line(text, first_line);
        let mut line_cursor = Cursor::new(text, first_line_offset);
        let mut soft_breaks = self.breaks.as_ref().map(|breaks|
            Cursor::new(breaks, first_line_offset)
        );

        let mut rendered_lines = Vec::new();
        for line_num in first_line..last_line {
            rendered_lines.push(self.render_line(tab_ctx, text,
                &mut line_cursor, soft_breaks.as_mut(), style_spans, line_num));
        }
        ops.push(self.build_update_op("ins", Some(rendered_lines), last_line - first_line));
        if last_line < height {
            if !self.dirty {
                ops.push(self.build_update_op("skip", None, last_line - first_line));
            }
            let op = if self.dirty { "invalidate" } else { "copy" };
            ops.push(self.build_update_op(op, None, height - last_line));
        }
        let params = json!({
            "ops": ops,
            "pristine": self.pristine,
        });
        tab_ctx.update_view(&self.view_id, &params);
        self.valid_lines.union_one_range(first_line, last_line);
    }


    /// Send lines within given region (plus slop) that the front-end does not already
    /// have.
    pub fn send_update_for_scroll<W: Write>(&mut self, text: &Rope, tab_ctx: &TabCtx<W>, style_spans: &Spans<Style>,
        first_line: usize, last_line: usize)
    {
        let first_line = max(first_line, SCROLL_SLOP) - SCROLL_SLOP;
        let last_line = last_line + SCROLL_SLOP;
        let height = self.offset_to_line_col(text, text.len()).0 + 1;
        let last_line = min(last_line, height);

        let mut ops = Vec::new();
        let mut line = 0;
        for (start, end) in self.valid_lines.minus_one_range(first_line, last_line) {
            // TODO: this has some duplication with send_update in the non-dirty case.
            if start > line {
                ops.push(self.build_update_op("copy", None, start - line));
            }
            let start_offset = self.offset_of_line(text, start);
            let mut line_cursor = Cursor::new(text, start_offset);
            let mut soft_breaks = self.breaks.as_ref().map(|breaks|
                Cursor::new(breaks, start_offset)
            );
            let mut rendered_lines = Vec::new();
            for line_num in start..end {
                rendered_lines.push(self.render_line(tab_ctx, text,
                                                     &mut line_cursor, soft_breaks.as_mut(),
                                                     style_spans, line_num));
            }
            ops.push(self.build_update_op("ins", Some(rendered_lines), end - start));
            ops.push(self.build_update_op("skip", None, end - start));
            line = end;
        }
        if line == 0 {
            // Front-end already has all lines, no need to send any more.
            return;
        }
        if line < height {
            ops.push(self.build_update_op("copy", None, height - line));
        }
        let params = json!({
            "ops": ops,
            "pristine": self.pristine,
        });
        tab_ctx.update_view(&self.view_id, &params);
        self.valid_lines.union_one_range(first_line, last_line);
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

    // If old-style selection is dirty, then copy it to (new) selection field.
    fn propagate_old_sel(&mut self) {
        if self.old_sel_dirty {
            self.selection.clear();
            let region = SelRegion {
                start: self.sel_start,
                end: self.sel_end,
                horiz: Some(self.cursor_col),
                affinity: Affinity::default(),
            };
            self.selection.add_region(region);
            self.old_sel_dirty = false;
            self.sel_dirty = true;
        }
    }

    // Update front-end with any changes to view since the last time sent.
    pub fn render_if_dirty<W: Write>(&mut self, text: &Rope, tab_ctx: &TabCtx<W>, style_spans: &Spans<Style>) {
        self.propagate_old_sel();
        if self.sel_dirty || self.dirty {
            let first_line = max(self.first_line, SCROLL_SLOP) - SCROLL_SLOP;
            let last_line = self.first_line + self.height + SCROLL_SLOP;
            self.send_update(text, tab_ctx, style_spans, first_line, last_line);
            self.sel_dirty = false;
            self.dirty = false;
        }
    }

    // TODO: finer grained tracking
    pub fn set_dirty(&mut self) {
        self.valid_lines.clear();
        self.dirty = true;
    }

    pub fn set_old_sel_dirty(&mut self) {
        self.old_sel_dirty = true;
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

    // Move up or down by `line_delta` lines and return offset where the
    // cursor lands.
    pub fn vertical_motion(&self, text: &Rope, line_delta: isize) -> usize {
        // This code is quite careful to avoid integer overflow.
        // TODO: write tests to verify
        let line = self.line_of_offset(text, self.sel_end);
        if line_delta < 0 && (-line_delta as usize) > line {
            return 0;
        }
        let line = if line_delta < 0 {
            line - (-line_delta as usize)
        } else {
            line.saturating_add(line_delta as usize)
        };
        let n_lines = self.line_of_offset(text, text.len());
        if line > n_lines {
            return text.len();
        }
        self.line_col_to_offset(text, line, self.cursor_col)
    }

    // use own breaks if present, or text if not (no line wrapping)

    fn line_of_offset(&self, text: &Rope, offset: usize) -> usize {
        match self.breaks {
            Some(ref breaks) => {
                breaks.convert_metrics::<BreaksBaseMetric, BreaksMetric>(offset)
            }
            None => text.line_of_offset(offset)
        }
    }

    /// Return the byte offset corresponding to the line `line`.
    fn offset_of_line(&self, text: &Rope, offset: usize) -> usize {
        match self.breaks {
            Some(ref breaks) => {
                breaks.convert_metrics::<BreaksMetric, BreaksBaseMetric>(offset)
            }
            None => text.offset_of_line(offset)
        }
    }

    pub fn rewrap(&mut self, text: &Rope, wrap_col: usize) {
        self.breaks = Some(linewrap::linewrap(text, wrap_col));
        self.wrap_col = wrap_col;
    }

    pub fn after_edit(&mut self, text: &Rope, delta: &Delta<RopeInfo>, pristine: bool) {
        let (iv, new_len) = delta.summary();
        // Note: this logic almost replaces setting the cursor in Editor::commit_delta,
        // but doesn't set col or scroll to the cursor. It could be extended to subsume
        // that entirely.
        // Also note: for committing plugin edits, we probably want to know the priority
        // of the delta so we can set the cursor before or after the edit, as needed.
        if self.sel_end >= iv.start() {
            if self.sel_end >= iv.end() {
                self.sel_end = self.sel_end - iv.size() + new_len;
            } else {
                self.sel_end = iv.start() + new_len;
            }
        }
        self.sel_start = self.sel_end;
        if self.breaks.is_some() {
            linewrap::rewrap(self.breaks.as_mut().unwrap(), text, iv, new_len, self.wrap_col);
        }
        self.pristine = pristine;
        self.dirty = true;
    }

    /// Call to mark view as pristine. Used after a buffer is saved.
    pub fn set_pristine(&mut self) {
        self.pristine = true;
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
