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

#[cfg(feature = "benchmarks")]
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

/// How tracing should be configured.
#[derive(Copy, Clone)]
pub struct Config {
    sample_limit_count: usize
}

impl Config {
    /// The maximum number of bytes the tracing data should take up.  This limit
    /// won't be exceeded by the underlying storage itself (i.e. rounds down).
    pub fn with_limit_bytes(size: usize) -> Self {
        Self::with_limit_count(size / size_of::<Sample>())
    }

    /// The maximum number of entries the tracing data should allow.  Total
    /// storage allocated will be limit * size_of<Sample>
    pub fn with_limit_count(limit: usize) -> Self {
        Self {
            sample_limit_count: limit
        }
    }

    /// The default amount of storage to allocate for tracing.  Currently 1 MB.
    pub fn default() -> Self {
        // 1 MB
        Self::with_limit_bytes(1 * 1024 * 1024)
    }

    /// The maximum amount of space the tracing data will take up.  This does
    /// not account for any overhead of storing the data itself (i.e. pointer to
    /// the heap, counters, etc); just the data itself.
    pub fn max_size_in_bytes(&self) -> usize {
        self.sample_limit_count * size_of::<Sample>()
    }

    /// The maximum number of samples that should be stored.
    pub fn max_samples(&self) -> usize {
        self.sample_limit_count
    }
}

static SAMPLE_COUNTER: AtomicUsize = ATOMIC_USIZE_INIT;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SampleType {
    /// This is an instantaneous sample (i.e. X occurred)
    Instant,
    /// This sample has a beginning & end to measure the time elapsed for a
    /// block of code.
    Duration,
}

/// Stores the relevant data about a sample for later serialization.
/// The payload associated with any sample is by default a string but may be
/// configured via the `dict_payload` or `json_payload` features (there is an
/// associated performance hit across the board for turning it on).
#[derive(Clone, Debug)]
pub struct Sample {
    /// A private ordering to apply to the events based on creation order.
    /// Disambiguates in case 2 samples might be created from different threads
    /// with the same start_ns for purposes of ordering.
    pub(crate) sample_id: usize,
    /// The name of the event to be shown.
    pub name: StrCow,
    /// List of categories the event applies to.
    pub categories: CategoriesT,
    /// An arbitrary payload to associate with the sample.  The type is
    /// controlled by features (default string).
    pub payload: Option<TracePayloadT>,
    /// When was the sample started.
    pub start_ns: u64,
    /// When the sample completed.  Equivalent to start_ns for instantaneous
    /// samples.  However, to distinguish instantaneous from duration samples
    /// look at the sample_type instead.
    end_ns: u64,
    /// Whether the sample was record via trace/trace_payload or
    /// trace_block/trace_closure.
    pub sample_type: SampleType,
    /// The thread the sample was captured on.
    pub tid: u64,
    /// The process the sample was captured in.
    pub pid: u64,
}

impl Sample {
    /// Constructs a Duration sample without an end timestamp set.  Should not
    /// be used directly.  Instead should be constructed via SampleGuard.
    pub fn new<S>(name: S, categories: CategoriesT, payload: Option<TracePayloadT>)
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

    /// Constructs an instantaneous sample.
    pub fn new_instant<S>(name: S, categories: CategoriesT,
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

    #[inline]
    pub fn set_end_ns(&mut self, end_ns: u64) {
        debug_assert_eq!(self.sample_type, SampleType::Duration, "invalid sample type {:?} doesn't have separate start/end", self.sample_type);
        debug_assert_eq!(self.end_ns, 0, "end timestamp already set");
        self.end_ns = end_ns;
    }

    #[inline]
    pub fn get_end_ns(&self) -> u64 {
        debug_assert_ne!(self.end_ns, 0, "end timestamp not set");
        debug_assert!(self.end_ns >= self.start_ns, "end timestamp is after begin: [{}, {})", self.start_ns, self.end_ns);
        self.end_ns
    }
}

impl PartialEq for Sample {
    fn eq(&self, other: &Sample) -> bool {
        self.pid == other.pid && self.sample_id == other.sample_id
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
        if self.pid == other.pid {
            self.sample_id.cmp(&other.sample_id)
        } else {
            self.start_ns.cmp(&other.start_ns)
        }
    }
}

impl Hash for Sample {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.pid, self.sample_id).hash(state);
    }
}

pub struct SampleGuard<'a> {
    sample: Option<Sample>,
    trace: Option<&'a Trace>,
}

impl<'a> SampleGuard<'a> {
    #[inline]
    fn new_disabled() -> Self {
        Self {
            sample: None,
            trace: None,
        }
    }

    #[inline]
    fn new<S>(trace: &'a Trace, name: S, categories: CategoriesT, payload: Option<TracePayloadT>)
        -> Self
        where S: Into<StrCow>
    {
        Self {
            sample: Some(Sample::new(name, categories, payload)),
            trace: Some(&trace),
        }
    }
}

impl<'a> Drop for SampleGuard<'a> {
    fn drop(&mut self) {
        if let Some(ref mut sample) = self.sample {
            sample.set_end_ns(time::precise_time_ns());
            self.trace.unwrap().record(sample);
        }
    }
}

/// Stores the tracing data.
pub struct Trace {
    enabled: AtomicBool,
    samples: Mutex<FixedLifoDeque<Sample>>,
}

impl Trace {
    pub fn disabled() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            samples: Mutex::new(FixedLifoDeque::new())
        }
    }

    pub fn enabled(config: Config) -> Self {
        Self {
            enabled: AtomicBool::new(true),
            samples: Mutex::new(FixedLifoDeque::with_limit(config.max_samples())),
        }
    }

    pub fn disable(&self) {
        let mut all_samples = self.samples.lock().unwrap();
        all_samples.reset_limit(0);
        self.enabled.store(false, AtomicOrdering::Relaxed);
    }

    #[inline]
    pub fn enable(&self) {
        self.enable_config(Config::default());
    }

    pub fn enable_config(&self, config: Config) {
        let mut all_samples = self.samples.lock().unwrap();
        all_samples.reset_limit(config.max_samples());
        self.enabled.store(true, AtomicOrdering::Relaxed);
    }

    /// Generally racy since the underlying storage might be mutated in a separate thread.
    /// Exposed for unit tests.
    pub fn get_samples_count(&self) -> usize {
        self.samples.lock().unwrap().len()
    }

    /// Exposed for unit tests only.
    pub fn get_samples_limit(&self) -> usize {
        self.samples.lock().unwrap().limit()
    }

    #[inline]
    pub(crate) fn record(&self, sample: &Sample) {
        let mut all_samples = self.samples.lock().unwrap();
        all_samples.push_back(sample.clone());
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(AtomicOrdering::Relaxed)
    }

    pub fn instant<S>(&self, name: S, categories: CategoriesT)
        where S: Into<StrCow>
    {
        if self.is_enabled() {
            self.record(&Sample::new_instant(name, categories, None));
        }
    }

    pub fn instant_payload<S, P>(&self, name: S, categories: CategoriesT, payload: P)
        where S: Into<StrCow>, P: Into<TracePayloadT>
    {
        if self.is_enabled() {
            self.record(&Sample::new_instant(name, categories, Some(payload.into())));
        }
    }

    pub fn block<'a, S>(&'a self, name: S, categories: CategoriesT) -> SampleGuard<'a>
        where S: Into<StrCow>
    {
        if !self.is_enabled() {
            SampleGuard::new_disabled()
        } else {
            SampleGuard::new(&self, name, categories, None)
        }
    }

    pub fn block_payload<'a, S, P>(&'a self, name: S, categories: CategoriesT, payload: P)
                               -> SampleGuard<'a>
        where S: Into<StrCow>, P: Into<TracePayloadT>
    {
        if !self.is_enabled() {
            SampleGuard::new_disabled()
        } else {
            SampleGuard::new(&self, name, categories, Some(payload.into()))
        }
    }

    pub fn closure<S, F, R>(&self, name: S, categories: CategoriesT, closure: F) -> R
        where S: Into<StrCow>, F: FnOnce() -> R
    {
        let _closure_guard = self.block(name, categories);
        let r = closure();
        r
    }

    pub fn closure_payload<S, P, F, R>(&self, name: S, categories: CategoriesT,
                                       closure: F, payload: P) -> R
        where S: Into<StrCow>, P: Into<TracePayloadT>,
              F: FnOnce() -> R
    {
        let _closure_guard = self.block_payload(name, categories, payload);
        let r = closure();
        r
    }

    pub fn samples_cloned_unsorted(&self) -> Vec<Sample> {
        let all_samples = self.samples.lock().unwrap();
        let mut as_vec = Vec::with_capacity(all_samples.len());
        as_vec.extend(all_samples.iter().cloned());
        as_vec
    }

    #[inline]
    pub fn samples_cloned_sorted(&self) -> Vec<Sample> {
        let mut samples = self.samples_cloned_unsorted();
        samples.sort_unstable();
        samples
    }
}

lazy_static! { static ref TRACE : Trace = Trace::disabled(); }

/// Enable tracing with the default configuration.  See Config::default.
/// Tracing is disabled initially on program launch.
#[inline]
pub fn enable_tracing() {
    TRACE.enable();
}

/// Enable tracing with a specific configuration. Tracing is disabled initially
/// on program launch.
#[inline]
pub fn enable_tracing_with_config(config: Config) {
    TRACE.enable_config(config);
}

/// Disable tracing.  This clears all trace data (& frees the memory).
#[inline]
pub fn disable_tracing() {
    TRACE.disable();
    SAMPLE_COUNTER.store(0, AtomicOrdering::Relaxed);
}

/// Is tracing enabled.  Technically doesn't guarantee any samples will be
/// stored as tracing could still be enabled but set with a limit of 0.
#[inline]
pub fn is_enabled() -> bool {
    TRACE.is_enabled()
}

/// Create an instantaneous sample without any payload.  This is the lowest
/// overhead tracing routine available.
///
/// # Performance
/// The `dict_payload` or `json_payload` feature makes this ~1.3-~1.5x slower.
/// See `trace_payload` for a more complete discussion.
///
/// # Arguments
///
/// * `name` - A string that provides some meaningful name to this sample.
/// Usage of static strings is encouraged for best performance to avoid copies.
/// However, anything that can be converted into a Cow string can be passed as
/// an argument.
///
/// * `categories` - A static array of static strings that tags the samples in
/// some way.
///
/// # Examples
///
/// ```
/// xi_trace::trace("something happened", &["rpc", "response"]);
/// ```
#[inline]
pub fn trace<S>(name: S, categories: CategoriesT)
    where S: Into<StrCow>
{
    TRACE.instant(name, categories);
}


/// Create an instantaneous sample with a payload.  The type the payload
/// conforms to is currently determined by the feature this library is compiled
/// with.  By default, the type is string-like just like name.  If compiled with
/// `dict_payload` then a Rust HashMap is expected while the `json_payload`
/// feature makes the payload a `serde_json::Value` (additionally the library
/// acquires a dependency on the `serde_json` crate.
///
/// # Performance
/// A static string has the lowest overhead as no copies are necessary, roughly
/// equivalent performance to a regular trace.  A string that needs to be copied
/// first can make it ~1.7x slower than a regular trace.
///
/// When compiling with `dict_payload` or `json_payload`, this is ~2.1x slower
/// than a string that needs to be copied (or ~4.5x slower than a static string)
///
/// # Arguments
///
/// * `name` - A string that provides some meaningful name to this sample.
/// Usage of static strings is encouraged for best performance to avoid copies.
/// However, anything that can be converted into a Cow string can be passed as
/// an argument.
///
/// * `categories` - A static array of static strings that tags the samples in
/// some way.
///
/// # Examples
///
/// ```
/// xi_trace::trace_payload("something happened", &["rpc", "response"], "a note about this");
/// ```
///
/// With `json_payload` feature:
/// ```
/// xi_trace::trace_payload("something happened", &["rpc", "response"], json!({"key": "value"}));
/// ```
#[inline]
pub fn trace_payload<S, P>(name: S, categories: CategoriesT, payload: P)
    where S: Into<StrCow>, P: Into<TracePayloadT>
{
    TRACE.instant_payload(name, categories, payload);
}

/// Creates a duration sample.  The sample is finalized (end_ns set) when the
/// returned value is dropped.  `trace_closure` may be prettier to read.
///
/// # Performance
/// See `trace_payload` for a more complete discussion.
///
/// # Arguments
///
/// * `name` - A string that provides some meaningful name to this sample.
/// Usage of static strings is encouraged for best performance to avoid copies.
/// However, anything that can be converted into a Cow string can be passed as
/// an argument.
///
/// * `categories` - A static array of static strings that tags the samples in
/// some way.
///
/// # Returns
/// A guard that when dropped will update the Sample with the timestamp & then
/// record it.
///
/// # Examples
///
/// ```
/// fn something_expensive() {
/// }
///
/// fn something_else_expensive() {
/// }
///
/// let trace_guard = xi_trace::trace_block("something_expensive", &["rpc", "request"]);
/// something_expensive();
/// std::mem::drop(trace_guard); // finalize explicitly if
///
/// {
///     let _guard = xi_trace::trace_block("something_else_expensive", &["rpc", "response"]);
///     something_else_expensive();
/// }
/// ```
#[inline]
pub fn trace_block<'a, S>(name: S, categories: CategoriesT) -> SampleGuard<'a>
    where S: Into<StrCow>
{
    TRACE.block(name, categories)
}


/// See `trace_block` for how the block works and `trace_payload` for a
/// discussion on payload.
#[inline]
pub fn trace_block_payload<'a, S, P>(name: S, categories: CategoriesT, payload: P)
    -> SampleGuard<'a>
    where S: Into<StrCow>, P: Into<TracePayloadT>
{
    TRACE.block_payload(name, categories, payload)
}

/// Creates a duration sample that measures how long the closure took to execute.
///
/// # Performance
/// See `trace_payload` for a more complete discussion.
///
/// # Arguments
///
/// * `name` - A string that provides some meaningful name to this sample.
/// Usage of static strings is encouraged for best performance to avoid copies.
/// However, anything that can be converted into a Cow string can be passed as
/// an argument.
///
/// * `categories` - A static array of static strings that tags the samples in
/// some way.
///
/// # Returns
/// The result of the closure.
///
/// # Examples
///
/// ```
/// fn something_expensive() -> u32 {
///     0
/// }
///
/// fn something_else_expensive(value: u32) {
/// }
///
/// let result = xi_trace::trace_closure("something_expensive", &["rpc", "request"], || {
///     something_expensive()
/// });
/// xi_trace::trace_closure("something_else_expensive", &["rpc", "response"], || {
///     something_else_expensive(result);
/// });
/// ```
#[inline]
pub fn trace_closure<S, F, R>(name: S, categories: CategoriesT, closure: F) -> R
    where S: Into<StrCow>, F: FnOnce() -> R
{
    TRACE.closure(name, categories, closure)
}

/// See `trace_closure` for how the closure works and `trace_payload` for a
/// discussion on payload.
#[inline]
pub fn trace_closure_payload<S, P, F, R>(name: S, categories: CategoriesT,
                                              closure: F, payload: P) -> R
    where S: Into<StrCow>, P: Into<TracePayloadT>,
          F: FnOnce() -> R
{
    TRACE.closure_payload(name, categories, closure, payload)
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
#[inline]
pub fn samples_cloned_unsorted() -> Vec<Sample> {
    TRACE.samples_cloned_unsorted()
}

/// Returns all the samples collected so far ordered chronologically by
/// creation.  Roughly corresponds to start_ns but instead there's a
/// monotonically increasing single global integer (when tracing) per creation
/// of Sample that determines order.
#[inline]
pub fn samples_cloned_sorted() -> Vec<Sample> {
    TRACE.samples_cloned_sorted()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "benchmarks")]
    use test::Bencher;
    #[cfg(feature = "benchmarks")]
    use test::black_box;

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

    #[test]
    fn test_samples_pulse() {
        let trace = Trace::enabled(Config::with_limit_count(10));
        for _i in 0..50 {
            trace.instant("test_samples_pulse", &["test"]);
        }
    }

    #[test]
    fn test_samples_block() {
        let trace = Trace::enabled(Config::with_limit_count(10));
        for _i in 0..50 {
            let _ = trace.block("test_samples_block", &["test"]);
        }
    }

    #[test]
    fn test_samples_closure() {
        let trace = Trace::enabled(Config::with_limit_count(10));
        for _i in 0..50 {
            trace.closure("test_samples_closure", &["test"], || {});
        }
    }

    #[test]
    fn test_disable_drops_all_samples() {
        let trace = Trace::enabled(Config::with_limit_count(10));
        assert_eq!(trace.is_enabled(), true);
        trace.instant("1", &["test"]);
        trace.instant("2", &["test"]);
        trace.instant("3", &["test"]);
        trace.instant("4", &["test"]);
        trace.instant("5", &["test"]);
        assert_eq!(trace.get_samples_count(), 5);
        assert_eq!(trace.samples_cloned_unsorted().len(), 5);
        trace.disable();
        assert_eq!(trace.get_samples_count(), 0);
    }

    #[test]
    fn test_get_samples() {
        let trace = Trace::enabled(Config::with_limit_count(20));
        assert_eq!(trace.samples_cloned_unsorted().len(), 0);

        assert_eq!(trace.is_enabled(), true);
        assert_eq!(trace.get_samples_limit(), 20);
        assert_eq!(trace.samples_cloned_unsorted().len(), 0);

        trace.closure_payload("x", &["test"], || (),
                              to_payload("test_get_samples"));
        assert_eq!(trace.get_samples_count(), 1);
        assert_eq!(trace.samples_cloned_unsorted().len(), 1);

        trace.closure_payload("y", &["test"], || {},
                              to_payload("test_get_samples"));
        assert_eq!(trace.samples_cloned_unsorted().len(), 2);

        trace.closure_payload("z", &["test"], || {},
                              to_payload("test_get_samples"));

        let snapshot = trace.samples_cloned_unsorted();
        assert_eq!(snapshot.len(), 3);

        assert_eq!(snapshot[0].name, "x");
        assert_eq!(snapshot[1].name, "y");
        assert_eq!(snapshot[2].name, "z");
    }

    #[test]
    fn test_trace_disabled() {
        let trace = Trace::disabled();
        assert_eq!(trace.get_samples_limit(), 0);
        assert_eq!(trace.get_samples_count(), 0);

        {
            trace.instant("something", &[]);
            let _x = trace.block("something", &[]);
            trace.closure("something", &[], || ());
        }

        assert_eq!(trace.get_samples_count(), 0);
    }

    #[test]
    fn test_get_samples_nested_trace() {
        let trace = Trace::enabled(Config::with_limit_count(11));
        assert_eq!(trace.is_enabled(), true);
        assert_eq!(trace.get_samples_limit(), 11);

        // current recording mechanism should see:
        // a, b, y, z, c, x
        // even though the actual sampling order (from timestamp of
        // creation) is:
        // x, a, y, b, z, c
        // This might be an over-specified test as it will
        // probably change as the recording internals change.
        trace.closure_payload("x", &["test"], || {
            trace.instant_payload("a", &["test"], to_payload("test_get_samples_nested_trace"));
            trace.closure_payload("y", &["test"], || {
                trace.instant_payload("b", &["test"], to_payload("test_get_samples_nested_trace"));
            }, to_payload("test_get_samples_nested_trace"));
            trace.block_payload("z", &["test"], to_payload("test_get_samples_nested_trace"));
            trace.instant_payload("c", &["test"], to_payload("test_get_samples_nested_trace"));
        }, to_payload("test_get_samples_nested_trace"));

        let snapshot = trace.samples_cloned_unsorted();
        assert_eq!(snapshot.len(), 6);

        assert_eq!(snapshot[0].name, "a");
        assert_eq!(snapshot[1].name, "b");
        assert_eq!(snapshot[2].name, "y");
        assert_eq!(snapshot[3].name, "z");
        assert_eq!(snapshot[4].name, "c");
        assert_eq!(snapshot[5].name, "x");
    }

    #[test]
    fn test_get_sorted_samples() {
        let trace = Trace::enabled(Config::with_limit_count(10));

        // current recording mechanism should see:
        // a, b, y, z, c, x
        // even though the actual sampling order (from timestamp of
        // creation) is:
        // x, a, y, b, z, c
        // This might be an over-specified test as it will
        // probably change as the recording internals change.
        trace.closure_payload("x", &["test"], || {
            trace.instant_payload("a", &["test"], to_payload("test_get_sorted_samples"));
            trace.closure_payload("y", &["test"], || {
                trace.instant_payload("b", &["test"], to_payload("test_get_sorted_samples"));
            }, to_payload("test_get_sorted_samples"));
            trace.block_payload("z", &["test"], to_payload("test_get_sorted_samples"));
            trace.instant("c", &["test"]);
        }, to_payload("test_get_sorted_samples"));

        let snapshot = trace.samples_cloned_sorted();
        assert_eq!(snapshot.len(), 6);

        assert_eq!(snapshot[0].name, "x");
        assert_eq!(snapshot[1].name, "a");
        assert_eq!(snapshot[2].name, "y");
        assert_eq!(snapshot[3].name, "b");
        assert_eq!(snapshot[4].name, "z");
        assert_eq!(snapshot[5].name, "c");
    }

    #[test]
    fn test_cross_process_samples() {
        let mut samples = vec![
            Sample::new_instant("local pid", &[], None),
            Sample::new_instant("remote pid", &[], None)];
        samples[0].pid = 1;
        samples[0].start_ns = 10;
        samples[0].end_ns = 10;

        samples[1].pid = 2;
        samples[1].start_ns = 5;
        samples[1].end_ns = 5;

        samples.sort();

        assert_eq!(samples[0].name, "remote pid");
        assert_eq!(samples[1].name, "local pid");
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_trace_instant_disabled(b: &mut Bencher) {
        let trace = Trace::disabled();

        b.iter(|| black_box(trace.instant("nothing", &["benchmark"])));
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_trace_instant(b: &mut Bencher) {
        let trace = Trace::enabled(Config::default());
        b.iter(|| black_box(trace.instant("something", &["benchmark"])));
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_trace_instant_with_payload(b: &mut Bencher) {
        let trace = Trace::enabled(Config::default());
        b.iter(|| black_box(trace.instant_payload(
            "something", &["benchmark"],
            to_payload("some description of the trace"))));
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_trace_block_disabled(b: &mut Bencher) {
        let trace = Trace::disabled();
        b.iter(|| black_box(trace.block("something", &["benchmark"])));
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_trace_block(b: &mut Bencher) {
        let trace = Trace::enabled(Config::default());
        b.iter(|| black_box(trace.block("something", &["benchmark"])));
    }


    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_trace_block_payload(b: &mut Bencher) {
        let trace = Trace::enabled(Config::default());
        b.iter(|| {
            black_box(trace.block_payload(
                    "something", &["benchmark"],
                    to_payload(("some payload for the block"))));
        });
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_trace_closure_disabled(b: &mut Bencher) {
        let trace = Trace::disabled();

        b.iter(|| black_box(trace.closure("something", &["benchmark"], || {})));
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_trace_closure(b: &mut Bencher) {
        let trace = Trace::enabled(Config::default());
        b.iter(|| black_box(trace.closure("something", &["benchmark"], || {})));
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_trace_closure_payload(b: &mut Bencher) {
        let trace = Trace::enabled(Config::default());
        b.iter(|| black_box(trace.closure_payload(
                    "something", &["benchmark"], || {},
                    to_payload(("some description of the closure")))));
    }

    // this is the cost contributed by the timestamp to trace()
    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_single_timestamp(b: &mut Bencher) {
        b.iter(|| black_box(time::precise_time_ns()));
    }

    // this is the cost contributed by the timestamp to
    // trace_block()/trace_closure
    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_two_timestamps(b: &mut Bencher) {
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
