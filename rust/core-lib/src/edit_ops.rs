// Copyright 2020 The xi-editor Authors.
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

//! Functions for editing ropes.

use std::collections::BTreeSet;

use xi_rope::{Cursor, DeltaBuilder, Interval, LinesMetric, Rope, RopeDelta, Transformer};

use crate::config::BufferItems;
use crate::line_offset::LineOffset;
use crate::selection::{Selection, SelRegion};
use crate::view::View;

/// Replaces the selection with the text `T`.
pub fn insert<T: Into<Rope>>(base : &Rope, regions: &[SelRegion], text: T) -> RopeDelta {
    let rope = text.into();
    let mut builder = DeltaBuilder::new(base.len());
    for region in regions {
        let iv = Interval::new(region.min(), region.max());
        builder.replace(iv, rope.clone());
    }
    return builder.build();
}

/// Leaves the current selection untouched, but surrounds it with two insertions.
pub fn surround<BT, AT>(base : &Rope, regions: &[SelRegion], before_text: BT, after_text: AT) -> RopeDelta
where
    BT: Into<Rope>,
    AT: Into<Rope>,
{
    let mut builder = DeltaBuilder::new(base.len());
    let before_rope = before_text.into();
    let after_rope = after_text.into();
    for region in regions {
        let before_iv = Interval::new(region.min(), region.min());
        builder.replace(before_iv, before_rope.clone());
        let after_iv = Interval::new(region.max(), region.max());
        builder.replace(after_iv, after_rope.clone());
    }
    return builder.build();
}

pub fn duplicate_line(base: &Rope, view: &View, config: &BufferItems) -> RopeDelta {
    let mut builder = DeltaBuilder::new(base.len());
    // get affected lines or regions
    let mut to_duplicate = BTreeSet::new();

    for region in view.sel_regions() {
        let (first_line, _) = view.offset_to_line_col(base, region.min());
        let line_start = view.offset_of_line(base, first_line);

        let mut cursor = match region.is_caret() {
            true => Cursor::new(base, line_start),
            false => {
                // duplicate all lines together that are part of the same selections
                let (last_line, _) = view.offset_to_line_col(base, region.max());
                let line_end = view.offset_of_line(base, last_line);
                Cursor::new(base, line_end)
            }
        };

        if let Some(line_end) = cursor.next::<LinesMetric>() {
            to_duplicate.insert((line_start, line_end));
        }
    }

    for (start, end) in to_duplicate {
        // insert duplicates
        let iv = Interval::new(start, start);
        builder.replace(iv, base.slice(start..end));

        // last line does not have new line character so it needs to be manually added
        if end == base.len() {
            builder.replace(iv, Rope::from(&config.line_ending))
        }
    }

    return builder.build();
    //self.this_edit_type = EditType::Other;
    //self.add_delta(builder.build());
}