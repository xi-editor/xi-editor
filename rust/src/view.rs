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
use serde_json::builder::ArrayBuilder;

use xi_rope::rope::Rope;

const SCROLL_SLOP: usize = 2;

pub struct View {
    pub sel_start: usize,
    pub sel_end: usize,
    first_line: usize,  // vertical scroll position
    height: usize,  // height of visible portion
}

impl View {
    pub fn new() -> View {
        View {
            sel_start: 0,
            sel_end: 0,
            first_line: 0,
            height: 10
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

    pub fn render_lines(&self, text: &Rope, first_line: usize, last_line: usize) -> Value {
        let mut builder = ArrayBuilder::new();
        let (cursor_line, cursor_col) = self.offset_to_line_col(text, self.sel_end);
        let sel_min_line = if self.sel_start == self.sel_end {
            cursor_line
        } else {
            text.line_of_offset(self.sel_min())
        };
        let sel_max_line = if self.sel_start == self.sel_end {
            cursor_line
        } else {
            text.line_of_offset(self.sel_max())
        };
        let mut line_num = first_line;
        for l in text.lines(text.offset_of_line(first_line), text.len()) {
            let mut line_builder = ArrayBuilder::new();
            let l_len = l.len();
            line_builder = line_builder.push(l);
            if line_num >= sel_min_line && line_num <= sel_max_line && self.sel_start != self.sel_end {
                let sel_start_ix = if line_num == sel_min_line {
                    self.sel_min() - text.offset_of_line(line_num)
                } else {
                    0
                };
                let sel_end_ix = if line_num == sel_max_line {
                    self.sel_max() - text.offset_of_line(line_num)
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
            }
            builder = builder.push(line_builder.unwrap());
            line_num += 1;
            if line_num == last_line {
                break;
            }
        }
        if line_num == cursor_line {
            builder = builder.push_array(|builder|
                builder.push("")
                    .push_array(|builder|
                        builder.push("cursor").push(0)));
        }
        builder.unwrap()
    }

    pub fn render(&self, text: &Rope, scroll_to: Option<usize>) -> Value {
        let first_line = max(self.first_line, SCROLL_SLOP) - SCROLL_SLOP;
        let last_line = self.first_line + self.height + SCROLL_SLOP;
        let lines = self.render_lines(text, first_line, last_line);
        let height = self.offset_to_line_col(text, text.len()).0 + 1;
        ArrayBuilder::new()
            .push("settext")
            .push_object(|builder| {
                let mut builder = builder
                    .insert("lines", lines)
                    .insert("first_line", first_line)
                    .insert("height", height);
                if let Some(scrollto) = scroll_to {
                    let (line, col) = self.offset_to_line_col(text, scrollto);
                    builder = builder.insert_array("scrollto", |builder|
                        builder.push(line).push(col));
                }
                builder
            })
            .unwrap()
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
        let line = text.line_of_offset(offset);
        (line, offset - text.offset_of_line(line))
    }

    pub fn line_col_to_offset(&self, text: &Rope, line: usize, col: usize) -> usize {
        let mut offset = text.offset_of_line(line).saturating_add(col);
        if offset >= text.len() {
            offset = text.len();
            if text.line_of_offset(offset) == line {
                return offset;
            }
        } else {
            // Snap to codepoint boundary
            offset = text.prev_codepoint_offset(offset + 1).unwrap();
        }

        // clamp to end of line
        let next_line_offset = text.offset_of_line(line + 1);
        if offset >= next_line_offset {
            offset = next_line_offset;
            if text.byte_at(offset - 1) == b'\n' {
                offset -= 1;
            }
        }
        if offset > 0 && text.byte_at(offset - 1) == b'\r' {
            offset -= 1;
        }
        offset
    }

    // Move up or down by `line_delta` lines and return offset where the
    // cursor lands. The `col` argument should probably move into the View
    // struct.
    pub fn vertical_motion(&self, text: &Rope, line_delta: isize, col: usize) -> usize {
        // This code is quite careful to avoid integer overflow.
        // TODO: write tests to verify
        let line = text.line_of_offset(self.sel_end);
        if line_delta < 0 && (-line_delta as usize) > line {
            return 0;
        }
        let line = if line_delta < 0 {
            line - (-line_delta as usize)
        } else {
            line.saturating_add(line_delta as usize)
        };
        let n_lines = text.line_of_offset(text.len());
        if line > n_lines {
            return text.len();
        }
        self.line_col_to_offset(text, line, col)
    }
}
