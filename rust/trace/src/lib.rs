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

#![cfg_attr(feature = "benchmarks", feature(test))]
#![cfg_attr(feature = "collections_range", feature(collections_range))]

#[macro_use]
extern crate lazy_static;
extern crate time;

extern crate libc;

#[cfg(all(test, feature = "benchmarks"))]
extern crate test;

#[cfg(feature = "json_payload")]
#[macro_use]
extern crate serde_json;

mod fixed_lifo_deque;
mod sys_pid;
mod sys_tid;

use std::borrow::Cow;
use std::cmp;

use std::mem::size_of;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicUsize, ATOMIC_USIZE_INIT, Ordering as AtomicOrdering};
use std::sync::Mutex;
use fixed_lifo_deque::FixedLifoDeque;

pub type StrCow = Cow<'static, str>;
pub type CategoriesT = &'static[&'static str];

#[cfg(all(not(feature = "dict_payload"), not(feature = "json_payload")))]
type TracePayloadT = StrCow;

#[cfg(feature = "json_payload")]
type TracePayloadT = serde_json::Value;

#[cfg(feature = "dict_payload")]
type TracePayloadT = std::collections::HashMap<StrCow, StrCow>;

#[derive(Copy, Clone)]
pub struct Config {
    /* Returns the maximum number of bytes that should be used for storing trace data */
    sample_limit_count: usize
}

impl Config {
    fn with_limit_bytes(size: usize) -> Self {
        Self::with_limit_count(size / size_of::<Sample>())
    }

    fn with_limit_count(limit: usize) -> Self {
        Self {
            sample_limit_count: limit
        }
    }

    fn default() -> Self {
        // 1 MB
        Self::with_limit_bytes(1 * 1024 * 1024)
    }

    pub fn max_size_in_bytes(&self) -> usize {
        self.sample_limit_count * size_of::<Sample>()
    }

    pub fn max_samples(&self) -> usize {
        self.sample_limit_count
    }
}

static SAMPLE_COUNTER: AtomicUsize = ATOMIC_USIZE_INIT;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SampleType {
    Instant,
    Duration,
}

#[derive(Clone, Debug)]
pub struct Sample {
    /// A private ordering to apply to the events based on creation order.
    /// Disambiguates in case 2 samples might be created from different threads
    /// with the same start_ns for purposes of ordering.
    sample_id: usize,
    /// The name of the event to be shown.
    pub name: StrCow,
    /// List of categories the event applies to.
    pub categories: CategoriesT,
    /// An arbitrary payload to associate with the sample.
    pub payload: Option<TracePayloadT>,
    /// When was the sample started.
    pub start_ns: u64,
    /// When the sample completed.  Equivalent to start_ns for instantaneous
    /// samples.
    pub end_ns: u64,
    /// Whether the sample was record via trace/trace_payload or
    /// trace_block/trace_closure.
    pub sample_type: SampleType,
    /// The thread the sample was captured on.
    pub tid: u64,
    /// The process the sample was captured in.
    pub pid: u64,
}

impl Sample {
    fn new<S>(name: S, categories: CategoriesT, payload: Option<TracePayloadT>)
        -> Self
        where S: Into<StrCow>
    {
        Self {
            sample_id: SAMPLE_COUNTER.fetch_add(1, AtomicOrdering::Relaxed),
            name: name.into(),
            categories: categories,
            start_ns: time::precise_time_ns(),
            payload: payload,
            end_ns: 0,
            sample_type: SampleType::Duration,
            tid: sys_tid::current_tid().unwrap(),
            pid: sys_pid::current_pid(),
        }
    }

    fn new_instant<S>(name: S, categories: CategoriesT,
                      payload: Option<TracePayloadT>) -> Self
        where S: Into<StrCow>
    {
        let now = time::precise_time_ns();
        Self {
            sample_id: SAMPLE_COUNTER.fetch_add(1, AtomicOrdering::Relaxed),
            name: name.into(),
            categories: categories,
            start_ns: now,
            payload: payload,
            end_ns: now,
            sample_type: SampleType::Instant,
            tid: sys_tid::current_tid().unwrap(),
            pid: sys_pid::current_pid(),
        }
    }
}

impl PartialEq for Sample {
    fn eq(&self, other: &Sample) -> bool {
        self.sample_id == other.sample_id
    }
}

impl Eq for Sample {}

impl PartialOrd for Sample {
    fn partial_cmp(&self, other: &Sample) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Sample {
    fn cmp(&self, other: &Sample) -> cmp::Ordering {
        self.sample_id.cmp(&other.sample_id)
    }
}

impl Hash for Sample {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.sample_id.hash(state);
    }
}

pub struct SampleGuard {
    sample: Option<Sample>,
}

impl SampleGuard {
    #[inline]
    fn new_disabled() -> Self {
        Self {
            sample: None
        }
    }

    #[inline]
    fn new<S>(name: S, categories: CategoriesT, payload: Option<TracePayloadT>)
        -> Self
        where S: Into<StrCow>
    {
        Self {
            sample: Some(Sample::new(name, categories, payload))
        }
    }
}

impl Drop for SampleGuard {
    fn drop(&mut self) {
        if let Some(ref mut sample) = self.sample {
            sample.end_ns = time::precise_time_ns();
            record_sample(sample);
        }
    }
}

struct Trace {
    enabled: AtomicBool,
    samples: Mutex<FixedLifoDeque<Sample>>,
}

impl Trace {
    fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            samples: Mutex::new(FixedLifoDeque::new())
        }
    }
}

lazy_static! { static ref TRACE : Trace = Trace::new(); }

pub fn enable_tracing() {
    enable_tracing_with_config(&Config::default());
}

pub fn enable_tracing_with_config(config: &Config) {
    let mut all_samples = TRACE.samples.lock().unwrap();
    all_samples.reset_limit(config.max_samples());
    TRACE.enabled.store(true, AtomicOrdering::Relaxed);
}

pub fn disable_tracing() {
    let mut all_samples = TRACE.samples.lock().unwrap();
    all_samples.reset_limit(0);
    SAMPLE_COUNTER.store(0, AtomicOrdering::Relaxed);
    TRACE.enabled.store(false, AtomicOrdering::Relaxed);
}

#[inline]
fn is_enabled() -> bool {
    TRACE.enabled.load(AtomicOrdering::Relaxed)
}

pub fn trace<S>(name: S, categories: CategoriesT)
    where S: Into<StrCow>
{
    if is_enabled() {
        record_sample(&Sample::new_instant(name, categories, None));
    }
}

pub fn trace_payload<S, P>(name: S, categories: CategoriesT, payload: P)
    where S: Into<StrCow>, P: Into<TracePayloadT>
{
    if is_enabled() {
        record_sample(&Sample::new_instant(name, categories,
                                           Some(payload.into())));
    }
}

pub fn trace_block<S>(name: S, categories: CategoriesT) -> SampleGuard
    where S: Into<StrCow>
{
    if !is_enabled() {
        SampleGuard::new_disabled()
    } else {
        SampleGuard::new(name, categories, None)
    }
}

pub fn trace_block_payload<S, P>(name: S, categories: CategoriesT, payload: P)
    -> SampleGuard
    where S: Into<StrCow>, P: Into<TracePayloadT>
{
    if !is_enabled() {
        SampleGuard::new_disabled()
    } else {
        SampleGuard::new(name, categories, Some(payload.into()))
    }
}

pub fn trace_closure<S, F, R>(name: S, categories: CategoriesT, closure: F) -> R
    where S: Into<StrCow>, F: FnOnce() -> R
{
    let _closure_guard = trace_block(name, categories);
    let r = closure();
    r
}

pub fn trace_closure_payload<S, P, F, R>(name: S, categories: CategoriesT,
                                              closure: F, payload: P) -> R
    where S: Into<StrCow>, P: Into<TracePayloadT>,
          F: FnOnce() -> R
{
    let _closure_guard = trace_block_payload(name, categories, payload);
    let r = closure();
    r
}

/// Returns all the samples collected so far.  There is no guarantee that the
/// samples are ordered chronologically for several reasons:
/// 1. Samples that span sections of code may be inserted on end instead of
/// beginning.
/// 2. Performance optimizations might have per-thread buffers.  Keeping all
/// that sorted would be prohibitively expensive.
/// 3. You may not care about them always being sorted if you're merging samples
/// from multiple distributed sources (i.e. you want to sort the merged result
/// rather than just this processe's samples).
pub fn samples_cloned_unsorted() -> Vec<Sample> {
    let all_samples = TRACE.samples.lock().unwrap();
    let mut as_vec = Vec::with_capacity(all_samples.len());
    as_vec.extend(all_samples.iter().cloned());
    as_vec
}

/// Returns all the samples collected so far ordered chronologically by
/// creation.  Roughly corresponds to start_ns but instead there's a
/// monotonically increasing single global integer (when tracing) per creation
/// of Sample that determines order.
pub fn samples_cloned_sorted() -> Vec<Sample> {
    let mut samples = samples_cloned_unsorted();
    samples.sort_unstable();
    samples
}

#[inline]
fn record_sample(sample: &Sample) {
    let mut all_samples = TRACE.samples.lock().unwrap();
    all_samples.push_back(sample.clone());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "benchmarks")]
    use test::Bencher;
    #[cfg(feature = "benchmarks")]
    use test::black_box;

    lazy_static! { static ref TEST_MUTEX : Mutex<u32> = Mutex::new(0); }

    #[cfg(all(not(feature = "dict_payload"), not(feature = "json_payload")))]
    fn to_payload(value: &'static str) -> &'static str {
        value
    }

    #[cfg(feature = "dict_payload")]
    fn to_payload(value: &'static str) -> TracePayloadT {
        let mut d = TracePayloadT::with_capacity(1);
        d.insert(StrCow::from("test"), StrCow::from(value));
        d
    }

    #[cfg(feature = "json_payload")]
    fn to_payload(value: &'static str) -> TracePayloadT {
        json!({"test": value})
    }

    fn get_samples_count() -> usize {
        TRACE.samples.lock().unwrap().len()
    }

    fn get_samples_limit() -> usize {
        TRACE.samples.lock().unwrap().limit()
    }

    #[test]
    fn test_samples_pulse() {
        let _test_mutex = TEST_MUTEX.lock().unwrap();

        disable_tracing();
        enable_tracing_with_config(&Config::with_limit_count(10));
        for _i in 0..50 {
            trace("test_samples_pulse", &["test"]);
        }
    }

    #[test]
    fn test_samples_block() {
        let _test_mutex = TEST_MUTEX.lock().unwrap();

        disable_tracing();
        enable_tracing_with_config(&Config::with_limit_count(10));
        for _i in 0..50 {
            let _ = trace_block("test_samples_block", &["test"]);
        }
    }

    #[test]
    fn test_samples_closure() {
        let _test_mutex = TEST_MUTEX.lock().unwrap();

        disable_tracing();
        enable_tracing_with_config(&Config::with_limit_count(10));
        for _i in 0..50 {
            trace_closure("test_samples_closure", &["test"], || {});
        }
    }

    #[test]
    fn test_disable_drops_all_samples() {
        let _test_mutex = TEST_MUTEX.lock().unwrap();

        disable_tracing();
        enable_tracing_with_config(&Config::with_limit_count(10));
        trace("1", &["test"]);
        trace("2", &["test"]);
        trace("3", &["test"]);
        trace("4", &["test"]);
        trace("5", &["test"]);
        assert_eq!(get_samples_count(), 5);
        assert_eq!(samples_cloned_unsorted().len(), 5);
        disable_tracing();
        assert_eq!(get_samples_count(), 0);
        assert_eq!(samples_cloned_unsorted().len(), 0);
    }

    #[test]
    fn test_get_samples() {
        let _test_mutex = TEST_MUTEX.lock().unwrap();

        disable_tracing();
        for i in 0..100 {
            assert_eq!(samples_cloned_unsorted().len(), 0, "i = {}", i);
        }

        enable_tracing_with_config(&Config::with_limit_count(20));
        assert_eq!(samples_cloned_unsorted().len(), 0);

        for i in 0..100 {
            assert_eq!(samples_cloned_unsorted().len(), 0, "i = {}", i);
        }

        assert_eq!(is_enabled(), true);
        assert_eq!(get_samples_limit(), 20);
        assert_eq!(samples_cloned_unsorted().len(), 0);

        trace_closure_payload("x", &["test"], || {},
                              to_payload("test_get_samples"));
        assert_eq!(samples_cloned_unsorted().len(), 1);

        trace_closure_payload("y", &["test"], || {},
                              to_payload("test_get_samples"));
        assert_eq!(samples_cloned_unsorted().len(), 2);

        trace_closure_payload("z", &["test"], || {},
                              to_payload("test_get_samples"));
        assert_eq!(samples_cloned_unsorted().len(), 3);

        let snapshot = samples_cloned_unsorted();
        assert_eq!(snapshot.len(), 3);
        assert_eq!(snapshot[0].sample_id, 0);
        assert_eq!(snapshot[0].name, "x");
        assert_eq!(snapshot[1].sample_id, 1);
        assert_eq!(snapshot[1].name, "y");
        assert_eq!(snapshot[2].sample_id, 2);
        assert_eq!(snapshot[2].name, "z");
    }

    #[test]
    fn test_get_samples_nested_trace() {
        let _test_mutex = TEST_MUTEX.lock().unwrap();

        disable_tracing();
        assert_eq!(get_samples_limit(), 0);

        enable_tracing_with_config(&Config::with_limit_count(11));
        assert_eq!(is_enabled(), true);
        assert_eq!(get_samples_limit(), 11);

        // current recording mechanism should see:
        // a, b, y, z, c, x
        // even though the actual sampling order (from timestamp of
        // creation) is:
        // x, a, y, b, z, c
        // This might be an over-specified test as it will
        // probably change as the recording internals change.
        trace_closure_payload("x", &["test"], || {
            trace_payload("a", &["test"], to_payload("test_get_samples_nested_trace"));
            trace_closure_payload("y", &["test"], || {
                trace_payload("b", &["test"], to_payload("test_get_samples_nested_trace"));
            }, to_payload("test_get_samples_nested_trace"));
            trace_block_payload("z", &["test"], to_payload("test_get_samples_nested_trace"));
            trace_payload("c", &["test"], to_payload("test_get_samples_nested_trace"));
        }, to_payload("test_get_samples_nested_trace"));

        let snapshot = samples_cloned_unsorted();
        assert_eq!(snapshot.len(), 6);

        assert_eq!(snapshot[0].sample_id, 1);
        assert_eq!(snapshot[0].name, "a");

        assert_eq!(snapshot[1].sample_id, 3);
        assert_eq!(snapshot[1].name, "b");

        assert_eq!(snapshot[2].sample_id, 2);
        assert_eq!(snapshot[2].name, "y");

        assert_eq!(snapshot[3].sample_id, 4);
        assert_eq!(snapshot[3].name, "z");

        assert_eq!(snapshot[4].sample_id, 5);
        assert_eq!(snapshot[4].name, "c");

        assert_eq!(snapshot[5].sample_id, 0);
        assert_eq!(snapshot[5].name, "x");
    }

    #[test]
    fn test_get_sorted_samples() {
        let _test_mutex = TEST_MUTEX.lock().unwrap();

        disable_tracing();
        enable_tracing_with_config(&Config::with_limit_count(10));

        // current recording mechanism should see:
        // a, b, y, z, c, x
        // even though the actual sampling order (from timestamp of
        // creation) is:
        // x, a, y, b, z, c
        // This might be an over-specified test as it will
        // probably change as the recording internals change.
        trace_closure_payload("x", &["test"], || {
            trace_payload("a", &["test"], to_payload("test_get_sorted_samples"));
            trace_closure_payload("y", &["test"], || {
                trace_payload("b", &["test"], to_payload("test_get_sorted_samples"));
            }, to_payload("test_get_sorted_samples"));
            trace_block_payload("z", &["test"], to_payload("test_get_sorted_samples"));
            trace("c", &["test"]);
        }, to_payload("test_get_sorted_samples"));

        let snapshot = samples_cloned_sorted();
        assert_eq!(snapshot.len(), 6);

        assert_eq!(snapshot[0].sample_id, 0);
        assert_eq!(snapshot[0].name, "x");

        assert_eq!(snapshot[1].sample_id, 1);
        assert_eq!(snapshot[1].name, "a");

        assert_eq!(snapshot[2].sample_id, 2);
        assert_eq!(snapshot[2].name, "y");

        assert_eq!(snapshot[3].sample_id, 3);
        assert_eq!(snapshot[3].name, "b");

        assert_eq!(snapshot[4].sample_id, 4);
        assert_eq!(snapshot[4].name, "z");

        assert_eq!(snapshot[5].sample_id, 5);
        assert_eq!(snapshot[5].name, "c");
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_trace_instant_disabled(b: &mut Bencher) {
        let _test_mutex = TEST_MUTEX.lock().unwrap();

        disable_tracing();
        b.iter(|| black_box(trace("nothing", &["benchmark"])));
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_trace_instant(b: &mut Bencher) {
        let _test_mutex = TEST_MUTEX.lock().unwrap();

        disable_tracing();
        enable_tracing_with_config(&Config::with_limit_count(500));
        b.iter(|| black_box(trace("something", &["benchmark"])));
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_trace_instant_with_payload(b: &mut Bencher) {
        let _test_mutex = TEST_MUTEX.lock().unwrap();

        disable_tracing();
        enable_tracing_with_config(&Config::with_limit_count(500));
        b.iter(|| black_box(trace_payload(
            "something", &["benchmark"],
            to_payload("some description of the trace"))));
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_trace_block_disabled(b: &mut Bencher) {
        let _test_mutex = TEST_MUTEX.lock().unwrap();

        disable_tracing();
        b.iter(|| black_box(trace_block("something", &["benchmark"])));
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_trace_block(b: &mut Bencher) {
        let _test_mutex = TEST_MUTEX.lock().unwrap();

        disable_tracing();
        enable_tracing_with_config(&Config::with_limit_count(500));
        b.iter(|| black_box(trace_block("something", &["benchmark"])));
    }


    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_trace_block_payload(b: &mut Bencher) {
        let _test_mutex = TEST_MUTEX.lock().unwrap();

        disable_tracing();
        enable_tracing_with_config(&Config::with_limit_count(500));
        b.iter(|| {
            black_box(trace_block_payload(
                    "something", &["benchmark"],
                    to_payload(("some payload for the block"))));
        });
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_trace_closure_disabled(b: &mut Bencher) {
        let _test_mutex = TEST_MUTEX.lock().unwrap();

        disable_tracing();
        b.iter(|| black_box(trace_closure("something", &["benchmark"], || {})));
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_trace_closure(b: &mut Bencher) {
        let _test_mutex = TEST_MUTEX.lock().unwrap();

        disable_tracing();
        enable_tracing_with_config(&Config::with_limit_count(500));
        b.iter(|| black_box(trace_closure("something", &["benchmark"], || {})));
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_trace_closure_payload(b: &mut Bencher) {
        let _test_mutex = TEST_MUTEX.lock().unwrap();

        disable_tracing();
        enable_tracing_with_config(&Config::with_limit_count(500));
        b.iter(|| black_box(trace_closure_payload(
                    "something", &["benchmark"], || {},
                    to_payload(("some description of the closure")))));
    }

    // this is the cost contributed by the timestamp to trace()
    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_single_timestamp(b: &mut Bencher) {
        let _test_mutex = TEST_MUTEX.lock().unwrap();

        b.iter(|| black_box(time::precise_time_ns()));
    }

    // this is the cost contributed by the timestamp to
    // trace_block()/trace_closure
    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_two_timestamps(b: &mut Bencher) {
        let _test_mutex = TEST_MUTEX.lock().unwrap();

        b.iter(|| {
            black_box(time::precise_time_ns());
            black_box(time::precise_time_ns());
        });
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_get_tid(b: &mut Bencher) {
        b.iter(|| black_box(sys_tid::current_tid()));
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_get_pid(b: &mut Bencher) {
        b.iter(|| sys_pid::current_pid());
    }
}
