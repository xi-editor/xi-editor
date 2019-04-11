// Copyright 2019 The xi-editor Authors.
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

use test::{black_box, Bencher};
use xi_rope::interval::Interval;
use xi_rope::spans::{Spans, SpansBuilder};

// generate num_spans spans with a seeded random composition, values from 0-134456
// for a given num_spans, it always generates the same spans
fn gen_spans(num_spans: usize) -> Spans<usize> {
    fn rng_gen() -> impl Fn(&mut usize) -> (Interval, usize) {
        let pseudo_random = |seed: &mut usize| -> usize {
            *seed = (8_121 * *seed + 28_411) % 134_456;
            *seed
        };

        let gen_range = move |seed: &mut usize| -> (Interval, usize) {
            let one = pseudo_random(seed);
            let two = pseudo_random(seed);
            let start = one.min(two);
            let end = one.max(two);
            (Interval::new(start, end), pseudo_random(seed))
        };

        gen_range
    }

    let mut seed = 123_456;
    let gen_range = rng_gen();

    let mut sb = SpansBuilder::new(134_456);
    let mut to_add: Vec<_> = (0..num_spans).map(|_| gen_range(&mut seed)).collect();
    to_add.sort_by(|p0, p1| p0.0.start().cmp(&p1.0.start()));
    for (iv, data) in to_add {
        sb.add_span(iv, data);
    }
    sb.build()
}

#[bench]
fn test_delete_intersecting_100_tiny_range(b: &mut Bencher) {
    let spans = gen_spans(100);
    b.iter(|| {
        let mut spans_copy = spans.clone();
        black_box(spans_copy.delete_intersecting(Interval::new(10_000, 10_020)))
    });
}

#[bench]
fn test_delete_intersecting_100_med_range(b: &mut Bencher) {
    let spans = gen_spans(100);
    b.iter(|| {
        let mut spans_copy = spans.clone();
        black_box(spans_copy.delete_intersecting(Interval::new(100, 10_000)))
    });
}

#[bench]
fn test_delete_intersecting_100_huge_range(b: &mut Bencher) {
    let spans = gen_spans(100);
    b.iter(|| {
        let mut spans_copy = spans.clone();
        black_box(spans_copy.delete_intersecting(Interval::new(100, 134_456)))
    });
}

#[bench]
fn test_delete_intersecting_10_000_tiny_range(b: &mut Bencher) {
    let spans = gen_spans(10_000);
    b.iter(|| {
        let mut spans_copy = spans.clone();
        black_box(spans_copy.delete_intersecting(Interval::new(10_000, 10_020)))
    });
}

#[bench]
fn test_delete_intersecting_10_000_med_range(b: &mut Bencher) {
    let spans = gen_spans(10_000);
    b.iter(|| {
        let mut spans_copy = spans.clone();
        black_box(spans_copy.delete_intersecting(Interval::new(100, 10_000)))
    });
}

#[bench]
fn test_delete_intersecting_10_000_huge_range(b: &mut Bencher) {
    let spans = gen_spans(10_000);
    b.iter(|| {
        let mut spans_copy = spans.clone();
        black_box(spans_copy.delete_intersecting(Interval::new(100, 134_456)))
    });
}

#[bench]
fn test_delete_intersecting_1_000_000_tiny_range(b: &mut Bencher) {
    let spans = gen_spans(1_000_000);
    b.iter(|| {
        let mut spans_copy = spans.clone();
        black_box(spans_copy.delete_intersecting(Interval::new(10_000, 10_020)))
    });
}

#[bench]
fn test_delete_intersecting_1_000_000_med_range(b: &mut Bencher) {
    let spans = gen_spans(1_000_000);
    b.iter(|| {
        let mut spans_copy = spans.clone();
        black_box(spans_copy.delete_intersecting(Interval::new(100, 10_000)))
    });
}

#[bench]
fn test_delete_intersecting_1_000_000_huge_range(b: &mut Bencher) {
    let spans = gen_spans(1_000_000);
    b.iter(|| {
        let mut spans_copy = spans.clone();
        black_box(spans_copy.delete_intersecting(Interval::new(100, 134_456)))
    });
}
