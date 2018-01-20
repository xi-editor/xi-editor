// Copyright 2018 Google LLC
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

#[macro_use]
extern crate criterion;

#[macro_use]
extern crate lazy_static;

use criterion::Criterion;
use std::sync::Mutex;

// FFI functions only work on the global singleton state.  Double-check that
// they're not parallelized by the runner (defensive - unlikely Criterion
// would do that as it would skew results).
lazy_static!(static ref BENCH_LOCK: Mutex<bool> = Mutex::new(false););

extern "C" {
    fn xi_trace_disable();
    fn xi_trace_enable();

    fn bench_trace_no_categories_iter();
    fn bench_trace_one_category_iter();
    fn bench_trace_two_categories_iter();
    fn bench_trace_block_no_categories_iter();
}

fn reset_enabled() {
    unsafe {
        xi_trace_disable();
        xi_trace_enable();
    }
}

fn bench_trace_no_categories(c: &mut Criterion) {
    let _locker = BENCH_LOCK.lock().unwrap();
    reset_enabled();
    c.bench_function("no categories", |b| b.iter(|| unsafe {bench_trace_no_categories_iter()}));
}

fn bench_trace_one_category(c: &mut Criterion) {
    let _locker = BENCH_LOCK.lock().unwrap();
    reset_enabled();
    c.bench_function("one category", |b| b.iter(|| unsafe {bench_trace_one_category_iter()}));
}

fn bench_trace_two_categories(c: &mut Criterion) {
    let _locker = BENCH_LOCK.lock().unwrap();
    reset_enabled();
    c.bench_function("two categories", |b| b.iter(|| unsafe {bench_trace_two_categories_iter()}));
}

fn bench_trace_block_no_categories(c: &mut Criterion) {
    let _locker = BENCH_LOCK.lock().unwrap();
    reset_enabled();
    c.bench_function("block no categories", |b| b.iter(|| unsafe {bench_trace_block_no_categories_iter()}));
}

criterion_group!(benches, bench_trace_no_categories, bench_trace_one_category, bench_trace_two_categories, bench_trace_block_no_categories);
criterion_main!(benches);

