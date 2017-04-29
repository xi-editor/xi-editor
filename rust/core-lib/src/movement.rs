// Copyright 2017 Google Inc. All rights reserved.
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

use selection::{Affinity, HorizPos, Selection, SelRegion};
use view::View;
use xi_rope::rope::Rope;

/// The specification of a movement.
#[derive(Clone, Copy)]
pub enum Movement {
    /// Move to the left by one grapheme cluster.
    Left,
    /// Move to the right by one grapheme cluster.
    Right,
}

/// Calculate a horizontal position in the view, based on the offset. Return
/// value has the same type as `region_movement` for convenience.
fn calc_horiz(view: &View, text: &Rope, offset: usize) -> (usize, Option<HorizPos>) {
    let (_line, col) = view.offset_to_line_col(text, offset);
    (offset, Some(col))
}

/// Compute the result of movement on one selection region.
fn region_movement(m: Movement, r: &SelRegion, view: &View, text: &Rope, modify: bool)
    -> (usize, Option<HorizPos>)
{
    match m {
        Movement::Left => {
            if r.is_caret() || modify {
                if let Some(offset) = text.prev_grapheme_offset(r.end) {
                    calc_horiz(view, text, offset)
                } else {
                    (0, None)
                }
            } else {
                calc_horiz(view, text, r.min())
            }
        }
        Movement::Right => {
            if r.is_caret() || modify {
                if let Some(offset) = text.next_grapheme_offset(r.end) {
                    calc_horiz(view, text, offset)
                } else {
                    (r.end, None)
                }
            } else {
                calc_horiz(view, text, r.max())
            }
        }
        //_ => (0, None)
    }
}

/// Compute a new selection by applying a movement to an existing selection.
///
/// In a multi-region selection, this function applies the movement to each
/// region in the selection, and returns the union of the results.
///
/// If `modify` is `true`, the selections are modified, otherwise the results
/// of individual region movements become carets.
pub fn selection_movement(m: Movement, s: &Selection, view: &View, text: &Rope,
    modify: bool) -> Selection
{
    let mut result = Selection::new();
    for r in s.iter() {
        let (offset, horiz) = region_movement(m, r, view, text, modify);
        let new_region = SelRegion {
            start: if modify { r.start } else { offset },
            end: offset,
            horiz: horiz,
            affinity: Affinity::default(),
        };
        result.add_region(new_region);
    }
    result
}
