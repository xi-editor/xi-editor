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

//! Representation and calculation of movement within a view.

use std::cmp::max;

use selection::{HorizPos, SelRegion, Selection};
use view::View;
use word_boundaries::WordCursor;
use xi_rope::rope::{LinesMetric, Rope};
use xi_rope::tree::Cursor;

/// The specification of a movement.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Movement {
    /// Move to the left by one grapheme cluster.
    Left,
    /// Move to the right by one grapheme cluster.
    Right,
    /// Move to the left by one word.
    LeftWord,
    /// Move to the right by one word.
    RightWord,
    /// Move to left end of visible line.
    LeftOfLine,
    /// Move to right end of visible line.
    RightOfLine,
    /// Move up one visible line.
    Up,
    /// Move down one visible line.
    Down,
    /// Move up one viewport height.
    UpPage,
    /// Move down one viewport height.
    DownPage,
    /// Move up to the next line that can preserve the cursor position.
    UpEnforceHorizPos,
    /// Move down to the next line that can preserve the cursor position.
    DownEnforceHorizPos,
    /// Move to the start of the text line.
    StartOfParagraph,
    /// Move to the end of the text line.
    EndOfParagraph,
    /// Move to the end of the text line, or next line if already at end.
    EndOfParagraphKill,
    /// Move to the start of the document.
    StartOfDocument,
    /// Move to the end of the document
    EndOfDocument,
}

/// Compute movement based on vertical motion by the given number of lines.
///
/// Note: in non-exceptional cases, this function preserves the `horiz`
/// field of the selection region.
fn vertical_motion(
    r: SelRegion,
    view: &View,
    text: &Rope,
    line_delta: isize,
    modify: bool,
) -> (usize, Option<HorizPos>) {
    // The active point of the selection
    let active = if modify {
        r.end
    } else if line_delta < 0 {
        r.min()
    } else {
        r.max()
    };
    let col = if let Some(col) = r.horiz { col } else { view.offset_to_line_col(text, active).1 };
    // This code is quite careful to avoid integer overflow.
    // TODO: write tests to verify
    let line = view.line_of_offset(text, active);
    let mut line_length = view.offset_of_line(text, line.saturating_add(1)) - view.offset_of_line(text, line);
    if line_delta < 0 && (-line_delta as usize) > line {
        if enforce_horiz_pos {
            return (active, Some(col));
        } else {
            return (0, Some(col));
        }
    }
    let mut line = if line_delta < 0 {
        line - (-line_delta as usize)
    } else {
        line.saturating_add(line_delta as usize)
    };
    let n_lines = view.line_of_offset(text, text.len());

    if !enforce_horiz_pos {
        if line > n_lines {
            return (text.len(), Some(col));
        }

        return (view.line_col_to_offset(text, line, col), Some(col));
    }

    // If the active columns is longer than the current line, use the current line length.
    let col = if line_length < col { line_length } else { col };

    loop {
        line_length = view.offset_of_line(text, line.saturating_add(1)) - view.offset_of_line(text, line);

        // If the line is longer than the current cursor position, break.
        // We use > instead of >= because line_length includes newline.
        if line_length > col { break; }

        // If you are trying to add a selection past the end of the file or before the first line, return original selection
        if line >= n_lines || (line == 0 && line_delta < 0) {
            return (active, Some(col));
        }

        line = if line_delta < 0 { line - 1 } else { line.saturating_add(1) };
    }

    (view.line_col_to_offset(text, line, col), Some(col))
}

/// When paging through a file, the number of lines from the previous page
/// that will also be visible in the next.
const SCROLL_OVERLAP: isize = 2;

/// Computes the actual desired amount of scrolling (generally slightly
/// less than the height of the viewport, to allow overlap).
fn scroll_height(view: &View) -> isize {
    max(view.scroll_height() as isize - SCROLL_OVERLAP, 1)
}

/// Compute the result of movement on one selection region.
pub fn region_movement(
    m: Movement,
    r: SelRegion,
    view: &View,
    text: &Rope,
    modify: bool,
) -> SelRegion {
    let (offset, horiz) = match m {
        Movement::Left => {
            if r.is_caret() || modify {
                if let Some(offset) = text.prev_grapheme_offset(r.end) {
                    (offset, None)
                } else {
                    (0, r.horiz)
                }
            } else {
                (r.min(), None)
            }
        }
        Movement::Right => {
            if r.is_caret() || modify {
                if let Some(offset) = text.next_grapheme_offset(r.end) {
                    (offset, None)
                } else {
                    (r.end, r.horiz)
                }
            } else {
                (r.max(), None)
            }
        }
        Movement::LeftWord => {
            let mut word_cursor = WordCursor::new(text, r.end);
            let offset = word_cursor.prev_boundary().unwrap_or(0);
            (offset, None)
        }
        Movement::RightWord => {
            let mut word_cursor = WordCursor::new(text, r.end);
            let offset = word_cursor.next_boundary().unwrap_or_else(|| text.len());
            (offset, None)
        }
        Movement::LeftOfLine => {
            let line = view.line_of_offset(text, r.end);
            let offset = view.offset_of_line(text, line);
            (offset, None)
        }
        Movement::RightOfLine => {
            let line = view.line_of_offset(text, r.end);
            let mut offset = text.len();

            // calculate end of line
            let next_line_offset = view.offset_of_line(text, line + 1);
            if line < view.line_of_offset(text, offset) {
                if let Some(prev) = text.prev_grapheme_offset(next_line_offset) {
                    offset = prev;
                }
            }
            (offset, None)
        }
        Movement::Up => vertical_motion(r, view, text, -1, modify, false),
        Movement::Down => vertical_motion(r, view, text, 1, modify, false),
        Movement::UpEnforceHorizPos => vertical_motion(r, view, text, -1, modify, true),
        Movement::DownEnforceHorizPos => vertical_motion(r, view, text, 1, modify, true),
        Movement::StartOfParagraph => {
            // Note: TextEdit would start at modify ? r.end : r.min()
            let mut cursor = Cursor::new(&text, r.end);
            let offset = cursor.prev::<LinesMetric>().unwrap_or(0);
            (offset, None)
        }
        Movement::EndOfParagraph => {
            // Note: TextEdit would start at modify ? r.end : r.max()
            let mut offset = r.end;
            let mut cursor = Cursor::new(&text, offset);
            if let Some(next_para_offset) = cursor.next::<LinesMetric>() {
                if cursor.is_boundary::<LinesMetric>() {
                    if let Some(eol) = text.prev_grapheme_offset(next_para_offset) {
                        offset = eol;
                    }
                } else if cursor.pos() == text.len() {
                    offset = text.len();
                }
            }
            (offset, None)
        }
        Movement::EndOfParagraphKill => {
            // Note: TextEdit would start at modify ? r.end : r.max()
            let mut offset = r.end;
            let mut cursor = Cursor::new(&text, offset);
            if let Some(next_para_offset) = cursor.next::<LinesMetric>() {
                offset = next_para_offset;
                if cursor.is_boundary::<LinesMetric>() {
                    if let Some(eol) = text.prev_grapheme_offset(next_para_offset) {
                        if eol != r.end {
                            offset = eol;
                        }
                    }
                }
            }
            (offset, None)
        }
        Movement::UpPage => vertical_motion(r, view, text, -scroll_height(view), modify, false),
        Movement::DownPage => vertical_motion(r, view, text, scroll_height(view), modify, false),
        Movement::StartOfDocument => (0, None),
        Movement::EndOfDocument => (text.len(), None),
    };
    SelRegion::new(if modify { r.start } else { offset }, offset).with_horiz(horiz)
}

/// Compute a new selection by applying a movement to an existing selection.
///
/// In a multi-region selection, this function applies the movement to each
/// region in the selection, and returns the union of the results.
///
/// If `modify` is `true`, the selections are modified, otherwise the results
/// of individual region movements become carets.
pub fn selection_movement(
    m: Movement,
    s: &Selection,
    view: &View,
    text: &Rope,
    modify: bool,
) -> Selection {
    let mut result = Selection::new();
    for &r in s.iter() {
        let new_region = region_movement(m, r, view, text, modify);
        result.add_region(new_region);
    }
    result
}
