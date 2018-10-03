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
extern crate xi_rope;

use test::Bencher;
use xi_rope::compare;

static EDITOR_STR: &str = include_str!("../../core-lib/src/editor.rs");
static VIEW_STR: &str = include_str!("../../core-lib/src/view.rs");

static INTERVAL_STR: &str = include_str!("../src/interval.rs");
static BREAKS_STR: &str = include_str!("../src/breaks.rs");

#[bench]
fn ne_idx_sw(b: &mut Bencher) {
    let one: String = [EDITOR_STR, VIEW_STR, INTERVAL_STR, BREAKS_STR].concat();
    let mut two = one.clone();
    unsafe {
        let b = two.as_bytes_mut();
        let idx = b.len() - 200;
        b[idx] = 0x02;
    }

    b.iter(|| {
        compare::ne_idx_fallback(one.as_bytes(), one.as_bytes());
        compare::ne_idx_fallback(one.as_bytes(), two.as_bytes());
    })
}

#[bench]
fn ne_idx_hw(b: &mut Bencher) {
    let one: String = [EDITOR_STR, VIEW_STR, INTERVAL_STR, BREAKS_STR].concat();
    let mut two = one.clone();
    unsafe {
        let b = two.as_bytes_mut();
        let idx = b.len() - 200;
        b[idx] = 0x02;
    }

    b.iter(|| {
        compare::ne_idx_simd(one.as_bytes(), one.as_bytes());
        compare::ne_idx_simd(one.as_bytes(), two.as_bytes());
    })
}

#[bench]
fn ne_idx_sw_rev(b: &mut Bencher) {
    let one: String = [EDITOR_STR, VIEW_STR, INTERVAL_STR, BREAKS_STR].concat();
    let mut two = one.clone();
    unsafe {
        let b = two.as_bytes_mut();
        let idx = 200;
        b[idx] = 0x02;
    }

    b.iter(|| {
        compare::ne_idx_rev_fallback(one.as_bytes(), one.as_bytes());
        compare::ne_idx_rev_fallback(one.as_bytes(), two.as_bytes());
    })
}

#[bench]
fn ne_idx_hw_rev(b: &mut Bencher) {
    let one: String = [EDITOR_STR, VIEW_STR, INTERVAL_STR, BREAKS_STR].concat();
    let mut two = one.clone();
    unsafe {
        let b = two.as_bytes_mut();
        let idx = 200;
        b[idx] = 0x02;
    }

    b.iter(|| {
        compare::ne_idx_rev_simd(one.as_bytes(), one.as_bytes());
        compare::ne_idx_rev_simd(one.as_bytes(), two.as_bytes());
    })
}
