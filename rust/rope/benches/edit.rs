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

#![feature(test)]

extern crate test;
extern crate xi_rope;

use test::Bencher;
use xi_rope::rope::Rope;


fn build_triangle(n: usize) -> String {
    let mut s = String::new();
    let mut line = String::new();
    for _ in 0 .. n {
        s += &line;
        s += "\n";
        line += "a";
    }
    s
}

fn build_short_lines(n: usize) -> String {
    let line = "match s.as_bytes()[minsplit - 1..splitpoint].iter().rposition(|&c| c == b'\n') {";
    let mut s = String::new();
    for _ in 0..n {
        s += line;
    }
    s
}

fn build_few_big_lines(size: usize) -> String {
    let mut s = String::with_capacity(size * 10 + 20);
    for _ in 0 .. 10 {
        for _ in 0..size {
            s += "a";
        }
        s += "\n";
    }
    s
}

#[bench]
fn benchmark_file_load_short_lines(b: &mut Bencher) {
    let text = build_short_lines(50_000);
    b.iter(|| { Rope::from(&text); });
}

#[bench]
fn benchmark_file_load_few_big_lines(b: &mut Bencher) {
    let text = build_few_big_lines(1_000_000);
    b.iter(|| { Rope::from(&text); });
}

#[bench]
fn benchmark_char_insertion_one_line_edit_str(b: &mut Bencher) {
    let mut text = Rope::from("b".repeat(100));
    let mut offset = 100;
    b.iter(|| {
        text.edit_str(offset..=offset, "a");
        offset += 1;
    });
}

#[bench]
fn benchmark_paste_into_line(b: &mut Bencher) {
    let mut text = Rope::from(build_short_lines(50_000));
    let insertion = "a".repeat(50);
    let mut offset = 100;
    b.iter(|| {
        text.edit_str(offset..=offset, &insertion);
        offset += 150;
    });
}

#[bench]
fn benchmark_insert_newline(b: &mut Bencher) {
    let mut text = Rope::from(build_few_big_lines(1_000_000));
    let mut offset = 1000;
    b.iter(|| {
        text.edit_str(offset..=offset, "\n");
        offset += 1001;
    });
}

#[bench]
fn benchmark_overwrite_into_line(b: &mut Bencher) {
    let mut text = Rope::from(build_short_lines(50_000));
    let mut offset = 100;
    let insertion = "a".repeat(50);
    b.iter(|| {
        // TODO: if the method runs too quickly, this may generate a fault
        // since there's an upper limit to how many times this can run.
        text.edit_str(offset..=offset + 20, &insertion);
        offset += 30;
    });
}

#[bench]
fn benchmark_triangle_concat_inplace(b: &mut Bencher) {
    let mut text = Rope::from("");
    let insertion = build_triangle(10_000);
    let insertion_len = insertion.len();
    let mut offset = 0;
    b.iter(|| {
        text.edit_str(offset..=offset, &insertion);
        offset += insertion_len;
    });
}
