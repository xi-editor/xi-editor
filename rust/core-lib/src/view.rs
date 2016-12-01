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

use serde_json::Value;
use serde_json::builder::{ArrayBuilder,ObjectBuilder};

use xi_rope::rope::{Rope, LinesMetric, RopeInfo};
use xi_rope::delta::{Delta};
use xi_rope::tree::Cursor;
use xi_rope::breaks::{Breaks, BreaksMetric, BreaksBaseMetric};
use xi_rope::interval::Interval;
use xi_rope::spans::{Spans, SpansBuilder};

use linewrap;

const SCROLL_SLOP: usize = 2;

#[derive(Default, Clone)]
pub struct Style {
    pub fg: u32,
    pub font_style: u8,  // same as syntect, 1 = bold, 2 = underline, 4 = italic
}

pub struct View {
    pub sel_start: usize,
    pub sel_end: usize,
    first_line: usize,  // vertical scroll position
    height: usize,  // height of visible portion
    breaks: Option<Breaks>,
    style_spans: Spans<Style>,
    cols: usize,
}

impl Default for View {
    fn default() -> View {
        View {
            sel_start: 0,
            sel_end: 0,
            first_line: 0,
            height: 10,
            breaks: None,
            style_spans: Spans::default(),
            cols: 0,
        }
    }
}

impl View {
    pub fn new() -> View {
        View::default()
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

    pub fn render_lines(&self, text: &Rope, first_line: usize, last_line: usize) -> Value {
        let mut builder = ArrayBuilder::new();
        let (cursor_line, cursor_col) = self.offset_to_line_col(text, self.sel_end);
        let sel_min_line = if self.sel_start == self.sel_end {
            cursor_line
        } else {
            self.line_of_offset(text, self.sel_min())
        };
        let sel_max_line = if self.sel_start == self.sel_end {
            cursor_line
        } else {
            self.line_of_offset(text, self.sel_max())
        };
        let first_line_offset = self.offset_of_line(text, first_line);
        let mut cursor = Cursor::new(text, first_line_offset);
        let mut breaks_cursor = self.breaks.as_ref().map(|breaks|
            Cursor::new(breaks, first_line_offset)
        );
        let mut line_num = first_line;
        loop {
            let mut line_builder = ArrayBuilder::new();
            let start_pos = cursor.pos();
            let pos = match breaks_cursor {
                Some(ref mut bc) => {
                    let pos = bc.next::<BreaksMetric>();
                    if let Some(pos) = pos {
                        cursor.set(pos);
                    }
                    pos
                }
                None => cursor.next::<LinesMetric>()
            };
            let mut is_last_line = false;
            let pos = match pos {
                Some(pos) => pos,
                None => {
                    is_last_line = true;
                    text.len()
                }
            };
            let l_str = text.slice_to_string(start_pos, pos);
            let l = &l_str;
            // TODO: strip trailing line end
            let l_len = l.len();
            line_builder = line_builder.push(l);
            line_builder = self.render_spans(line_builder, start_pos, pos);
            if line_num >= sel_min_line && line_num <= sel_max_line && self.sel_start != self.sel_end {
                let sel_start_ix = if line_num == sel_min_line {
                    self.sel_min() - self.offset_of_line(text, line_num)
                } else {
                    0
                };
                let sel_end_ix = if line_num == sel_max_line {
                    self.sel_max() - self.offset_of_line(text, line_num)
                } else {
                    l_len
                };
                line_builder = line_builder.push_array(|builder|
                    builder.push("sel")
                        .push(sel_start_ix)
                        .push(sel_end_ix)
                );
            }
            if line_num == cursor_line {
                line_builder = line_builder.push_array(|builder|
                    builder.push("cursor")
                        .push(cursor_col)
                );
            }            builder = builder.push(line_builder.build());
            line_num += 1;
            if is_last_line || line_num == last_line {
                break;
            }
        }
        builder.build()
    }

    pub fn render_spans(&self, mut builder: ArrayBuilder, start: usize, end: usize) -> ArrayBuilder {
        let style_spans = self.style_spans.subseq(Interval::new_closed_open(start, end));
        for (iv, style) in style_spans.iter() {
            builder = builder.push_array(|builder|
                builder.push("fg")
                    .push(iv.start())
                    .push(iv.end())
                    .push(style.fg)
                    .push(style.font_style));
        }
        builder
    }

    pub fn render(&self, text: &Rope, scroll_to: Option<usize>) -> Value {
        let first_line = max(self.first_line, SCROLL_SLOP) - SCROLL_SLOP;
        let last_line = self.first_line + self.height + SCROLL_SLOP;
        let lines = self.render_lines(text, first_line, last_line);
        let height = self.offset_to_line_col(text, text.len()).0 + 1;
        let mut builder = ObjectBuilder::new()
            .insert("lines", lines)
            .insert("first_line", first_line)
            .insert("height", height);
        if let Some(scrollto) = scroll_to {
            let (line, col) = self.offset_to_line_col(text, scrollto);
            builder = builder.insert_array("scrollto", |builder|
                builder.push(line).push(col));
        }
        builder.build()
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
    // cursor lands. The `col` argument should probably move into the View
    // struct.
    pub fn vertical_motion(&self, text: &Rope, line_delta: isize, col: usize) -> usize {
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
        self.line_col_to_offset(text, line, col)
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

    fn offset_of_line(&self, text: &Rope, offset: usize) -> usize {
        match self.breaks {
            Some(ref breaks) => {
                breaks.convert_metrics::<BreaksMetric, BreaksBaseMetric>(offset)
            }
            None => text.offset_of_line(offset)
        }
    }

    pub fn rewrap(&mut self, text: &Rope, cols: usize) {
        self.breaks = Some(linewrap::linewrap(text, cols));
        self.cols = cols;
    }

    pub fn after_edit(&mut self, text: &Rope, delta: &Delta<RopeInfo>) {
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
            linewrap::rewrap(self.breaks.as_mut().unwrap(), text, iv, new_len, self.cols);
        }
        // TODO: maybe more precise editing based on actual delta rather than summary.
        // TODO: perhaps use different semantics for spans that enclose the edited region.
        // Currently it breaks any such span in half and applies no spans to the inserted
        // text. That's ok for syntax highlighting but not ideal for rich text.
        let empty_spans = SpansBuilder::new(new_len).build();
        self.style_spans.edit(iv, empty_spans);
    }

    pub fn reset_breaks(&mut self) {
        self.breaks = None;
    }

    pub fn set_test_fg_spans(&mut self) {
        let mut sb = SpansBuilder::new(15);
        let style = Style { fg: 0xffc00000, font_style: 0 };
        sb.add_span(Interval::new_closed_open(5, 10), style);
        self.style_spans = sb.build();
    }

    pub fn set_fg_spans(&mut self, start: usize, end: usize, spans: Spans<Style>) {
        self.style_spans.edit(Interval::new_closed_closed(start, end), spans);
    }
}
