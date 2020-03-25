// Copyright 2018 The xi-editor Authors.
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

#![feature(test)]

extern crate test;
extern crate xi_core_lib as xi_core;
extern crate xi_rope;

use crate::xi_core::line_offset::LineOffset;
use crate::xi_core::tabs::BufferId;
use crate::xi_core::view::View;
use test::Bencher;
use xi_rope::Rope;

fn build_short_lines(n: usize) -> String {
    let line =
        "See it, the beautiful ball Poised in the toyshop window, Rounder than sun or moon.\n";
    let mut s = String::new();
    for _ in 0..n {
        s += line;
    }
    s
}

#[bench]
fn line_of_offset_no_breaks(b: &mut Bencher) {
    let text = Rope::from(build_short_lines(10_000));
    let view = View::new(1.into(), BufferId::new(2));

    let total_bytes = text.len();
    b.iter(|| {
        for i in 0..total_bytes {
            let _line = view.line_of_offset(&text, i);
        }
    })
}

#[bench]
fn line_of_offset_col_breaks(b: &mut Bencher) {
    let text = Rope::from(build_short_lines(10_000));
    let mut view = View::new(1.into(), BufferId::new(2));
    view.debug_force_rewrap_cols(&text, 20);

    let total_bytes = text.len();
    b.iter(|| {
        for i in 0..total_bytes {
            let _line = view.line_of_offset(&text, i);
        }
    })
}

#[bench]
fn offset_of_line_no_breaks(b: &mut Bencher) {
    let text = Rope::from(build_short_lines(10_000));
    let view = View::new(1.into(), BufferId::new(2));

    b.iter(|| {
        for i in 0..10_000 {
            let _line = view.offset_of_line(&text, i);
        }
    })
}

#[bench]
fn offset_of_line_col_breaks(b: &mut Bencher) {
    let text = Rope::from(build_short_lines(10_000));
    let mut view = View::new(1.into(), BufferId::new(2));
    view.debug_force_rewrap_cols(&text, 20);

    b.iter(|| {
        for i in 0..10_000 {
            let _line = view.offset_of_line(&text, i);
        }
    })
}
