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

#![feature(test)]

extern crate test;
extern crate xi_rope;

use test::Bencher;
use xi_rope::tree::*;

fn build_triangle(n: usize) -> String {
    let mut s = String::new();
    let mut line = String::new();
    for _ in 0..n {
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
    for _ in 0..10 {
        for _ in 0..size {
            s += "a";
        }
        s += "\n";
    }
    s
}

#[bench]
fn build_tree_triangle(b: &mut Bencher) {
    let mut t = TreeBuilder::new();
    b.iter(|| t.push_str(&build_triangle(500)));
}

#[bench]
fn build_tree_short_lines(b: &mut Bencher) {
    let mut t = TreeBuilder::new();
    b.iter(|| t.push_str(&build_short_lines(1_000)));
}

#[bench]
fn build_tree_few_short_lines(b: &mut Bencher) {
    let mut t = TreeBuilder::new();
    b.iter(|| t.push_str(&build_few_big_lines(1_000)));
}

#[bench]
fn build_tree_triangle_stack(b: &mut Bencher) {
    let mut t = TreeBuilder::new();
    b.iter(|| t.push_str_stacked(&build_triangle(500)));
}

#[bench]
fn build_tree_short_lines_stack(b: &mut Bencher) {
    let mut t = TreeBuilder::new();
    b.iter(|| t.push_str_stacked(&build_short_lines(1_000)));
}

#[bench]
fn build_tree_few_short_lines_stack(b: &mut Bencher) {
    let mut t = TreeBuilder::new();
    b.iter(|| t.push_str_stacked(&build_few_big_lines(1_000)));
}
