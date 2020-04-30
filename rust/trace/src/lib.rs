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
#![cfg_attr(feature = "benchmarks", feature(test))]
#![allow(clippy::identity_op, clippy::new_without_default, clippy::trivially_copy_pass_by_ref)]

#[macro_use]
extern crate lazy_static;
extern crate time;

#[macro_use]
extern crate serde_derive;

extern crate serde;

#[macro_use]
extern crate log;

extern crate libc;

#[cfg(feature = "benchmarks")]
extern crate test;

#[cfg(any(test, feature = "json_payload", feature = "chroma_trace_dump"))]
#[cfg_attr(any(test), macro_use)]
extern crate serde_json;

mod fixed_lifo_deque;
mod sys_pid;
mod sys_tid;

#[cfg(feature = "chrome_trace_event")]
pub mod chrome_trace_dump;

use crate::fixed_lifo_deque::FixedLifoDeque;
use std::borrow::Cow;
use std::cmp;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::hash::{Hash, Hasher};
use std::mem::size_of;
use std::path::Path;
use std::string::ToString;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Mutex;

pub type StrCow = Cow<'static, str>;

#[derive(Clone, Debug)]
pub enum CategoriesT {
    StaticArray(&'static [&'static str]),
    DynamicArray(Vec<String>),
}

trait StringArrayEq<Rhs: ?Sized = Self> {
    fn arr_eq(&self, other: &Rhs) -> bool;
}

impl StringArrayEq<[&'static str]> for Vec<String> {
    fn arr_eq(&self, other: &[&'static str]) -> bool {
        if self.len() != other.len() {
            return false;
        }

        for i in 0..self.len() {
            if self[i] != other[i] {
                return false;
            }
        }
        true
    }
}

impl StringArrayEq<Vec<String>> for &'static [&'static str] {
    fn arr_eq(&self, other: &Vec<String>) -> bool {
        if self.len() != other.len() {
            return false;
        }
        for i in 0..self.len() {
            if self[i] != other[i] {
                return false;
            }
        }
        true
    }
}

impl PartialEq for CategoriesT {
    fn eq(&self, other: &CategoriesT) -> bool {
        match *self {
            CategoriesT::StaticArray(ref self_arr) => match *other {
                CategoriesT::StaticArray(ref other_arr) => self_arr.eq(other_arr),
                CategoriesT::DynamicArray(ref other_arr) => self_arr.arr_eq(other_arr),
            },
            CategoriesT::DynamicArray(ref self_arr) => match *other {
                CategoriesT::StaticArray(ref other_arr) => self_arr.arr_eq(other_arr),
                CategoriesT::DynamicArray(ref other_arr) => self_arr.eq(other_arr),
            },
        }
    }
}

impl Eq for CategoriesT {}

impl serde::Serialize for CategoriesT {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.join(",").serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for CategoriesT {
    fn deserialize<D>(deserializer: D) -> Result<CategoriesT, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Visitor;
        struct CategoriesTVisitor;

        impl<'de> Visitor<'de> for CategoriesTVisitor {
            type Value = CategoriesT;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("comma-separated strings")
            }

            fn visit_str<E>(self, v: &str) -> Result<CategoriesT, E>
            where
                E: serde::de::Error,
            {
                let categories = v.split(',').map(ToString::to_string).collect();
                Ok(CategoriesT::DynamicArray(categories))
            }
        }

        deserializer.deserialize_str(CategoriesTVisitor)
    }
}

impl CategoriesT {
    pub fn join(&self, sep: &str) -> String {
        match *self {
            CategoriesT::StaticArray(ref arr) => arr.join(sep),
            CategoriesT::DynamicArray(ref vec) => vec.join(sep),
        }
    }
}

macro_rules! categories_from_constant_array {
    ($num_args: expr) => {
        impl From<&'static [&'static str; $num_args]> for CategoriesT {
            fn from(c: &'static [&'static str; $num_args]) -> CategoriesT {
                CategoriesT::StaticArray(c)
            }
        }
    };
}

categories_from_constant_array!(0);
categories_from_constant_array!(1);
categories_from_constant_array!(2);
categories_from_constant_array!(3);
categories_from_constant_array!(4);
categories_from_constant_array!(5);
categories_from_constant_array!(6);
categories_from_constant_array!(7);
categories_from_constant_array!(8);
categories_from_constant_array!(9);
categories_from_constant_array!(10);

impl From<Vec<String>> for CategoriesT {
    fn from(c: Vec<String>) -> CategoriesT {
        CategoriesT::DynamicArray(c)
    }
}

#[cfg(not(feature = "json_payload"))]
pub type TracePayloadT = StrCow;

#[cfg(feature = "json_payload")]
pub type TracePayloadT = serde_json::Value;

/// How tracing should be configured.
#[derive(Copy, Clone)]
pub struct Config {
    sample_limit_count: usize,
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
        Self { sample_limit_count: limit }
    }

    /// The default amount of storage to allocate for tracing.  Currently 1 MB.
    pub fn default() -> Self {
        // 1 MB
        Self::with_limit_bytes(1 * 1024 * 1024)
    }

    /// The maximum amount of space the tracing data will take up.  This does
    /// not account for any overhead of storing the data itself (i.e. pointer to
    /// the heap, counters, etc); just the data itself.
    pub fn max_size_in_bytes(self) -> usize {
        self.sample_limit_count * size_of::<Sample>()
    }

    /// The maximum number of samples that should be stored.
    pub fn max_samples(self) -> usize {
        self.sample_limit_count
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleEventType {
    DurationBegin,
    DurationEnd,
    CompleteDuration,
    Instant,
    AsyncStart,
    AsyncInstant,
    AsyncEnd,
    FlowStart,
    FlowInstant,
    FlowEnd,
    ObjectCreated,
    ObjectSnapshot,
    ObjectDestroyed,
    Metadata,
}

impl SampleEventType {
    // TODO(vlovich): Replace all of this with serde flatten + rename once
    // https://github.com/serde-rs/serde/issues/1189 is fixed.
    #[inline]
    fn into_chrome_id(self) -> char {
        match self {
            SampleEventType::DurationBegin => 'B',
            SampleEventType::DurationEnd => 'E',
            SampleEventType::CompleteDuration => 'X',
            SampleEventType::Instant => 'i',
            SampleEventType::AsyncStart => 'b',
            SampleEventType::AsyncInstant => 'n',
            SampleEventType::AsyncEnd => 'e',
            SampleEventType::FlowStart => 's',
            SampleEventType::FlowInstant => 't',
            SampleEventType::FlowEnd => 'f',
            SampleEventType::ObjectCreated => 'N',
            SampleEventType::ObjectSnapshot => 'O',
            SampleEventType::ObjectDestroyed => 'D',
            SampleEventType::Metadata => 'M',
        }
    }

    #[inline]
    fn from_chrome_id(symbol: char) -> Self {
        match symbol {
            'B' => SampleEventType::DurationBegin,
            'E' => SampleEventType::DurationEnd,
            'X' => SampleEventType::CompleteDuration,
            'i' => SampleEventType::Instant,
            'b' => SampleEventType::AsyncStart,
            'n' => SampleEventType::AsyncInstant,
            'e' => SampleEventType::AsyncEnd,
            's' => SampleEventType::FlowStart,
            't' => SampleEventType::FlowInstant,
            'f' => SampleEventType::FlowEnd,
            'N' => SampleEventType::ObjectCreated,
            'O' => SampleEventType::ObjectSnapshot,
            'D' => SampleEventType::ObjectDestroyed,
            'M' => SampleEventType::Metadata,
            _ => panic!("Unexpected chrome sample type '{}'", symbol),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MetadataType {
    ProcessName {
        name: String,
    },
    #[allow(dead_code)]
    ProcessLabels {
        labels: String,
    },
    #[allow(dead_code)]
    ProcessSortIndex {
        sort_index: i32,
    },
    ThreadName {
        name: String,
    },
    #[allow(dead_code)]
    ThreadSortIndex {
        sort_index: i32,
    },
}

impl MetadataType {
    fn sample_name(&self) -> &'static str {
        match *self {
            MetadataType::ProcessName { .. } => "process_name",
            MetadataType::ProcessLabels { .. } => "process_labels",
            MetadataType::ProcessSortIndex { .. } => "process_sort_index",
            MetadataType::ThreadName { .. } => "thread_name",
            MetadataType::ThreadSortIndex { .. } => "thread_sort_index",
        }
    }

    fn consume(self) -> (Option<String>, Option<i32>) {
        match self {
            MetadataType::ProcessName { name } => (Some(name), None),
            MetadataType::ThreadName { name } => (Some(name), None),
            MetadataType::ProcessSortIndex { sort_index } => (None, Some(sort_index)),
            MetadataType::ThreadSortIndex { sort_index } => (None, Some(sort_index)),
            MetadataType::ProcessLabels { .. } => (None, None),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SampleArgs {
    /// An arbitrary payload to associate with the sample.  The type is
    /// controlled by features (default string).
    #[serde(rename = "xi_payload")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<TracePayloadT>,

    /// The name to associate with the pid/tid.  Whether it's associated with
    /// the pid or the tid depends on the name of the event
    /// via process_name/thread_name respectively.
    #[serde(rename = "name")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_name: Option<StrCow>,

    /// Sorting priority between processes/threads in the view.
    #[serde(rename = "sort_index")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_sort_index: Option<i32>,
}

#[inline]
fn ns_to_us(ns: u64) -> u64 {
    ns / 1000
}

//NOTE: serde requires this to take a reference
fn serialize_event_type<S>(ph: &SampleEventType, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    s.serialize_char(ph.into_chrome_id())
}

fn deserialize_event_type<'de, D>(d: D) -> Result<SampleEventType, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(d).map(SampleEventType::from_chrome_id)
}

/// Stores the relevant data about a sample for later serialization.
/// The payload associated with any sample is by default a string but may be
/// configured via the `json_payload` feature (there is an
/// associated performance hit across the board for turning it on).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Sample {
    /// The name of the event to be shown.
    pub name: StrCow,
    /// List of categories the event applies to.
    #[serde(rename = "cat")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<CategoriesT>,
    /// When was the sample started.
    #[serde(rename = "ts")]
    pub timestamp_us: u64,
    /// What kind of sample this is.
    #[serde(rename = "ph")]
    #[serde(serialize_with = "serialize_event_type")]
    #[serde(deserialize_with = "deserialize_event_type")]
    pub event_type: SampleEventType,
    #[serde(rename = "dur")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_us: Option<u64>,
    /// The process the sample was captured in.
    pub pid: u64,
    /// The thread the sample was captured on.  Omitted for Metadata events that
    /// want to set the process name (if provided then sets the thread name).
    pub tid: u64,
    #[serde(skip_serializing)]
    pub thread_name: Option<StrCow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<SampleArgs>,
}

fn to_cow_str<S>(s: S) -> StrCow
where
    S: Into<StrCow>,
{
    s.into()
}

impl Sample {
    fn thread_name() -> Option<StrCow> {
        let thread = std::thread::current();
        thread.name().map(|ref s| to_cow_str((*s).to_string()))
    }

    /// Constructs a Begin or End sample.  Should not be used directly.  Instead
    /// should be constructed via SampleGuard.
    pub fn new_duration_marker<S, C>(
        name: S,
        categories: C,
        payload: Option<TracePayloadT>,
        event_type: SampleEventType,
    ) -> Self
    where
        S: Into<StrCow>,
        C: Into<CategoriesT>,
    {
        Self {
            name: name.into(),
            categories: Some(categories.into()),
            timestamp_us: ns_to_us(time::precise_time_ns()),
            event_type,
            duration_us: None,
            tid: sys_tid::current_tid().unwrap(),
            thread_name: Sample::thread_name(),
            pid: sys_pid::current_pid(),
            args: Some(SampleArgs { payload, metadata_name: None, metadata_sort_index: None }),
        }
    }

    /// Constructs a Duration sample.  For use via xi_trace::closure.
    pub fn new_duration<S, C>(
        name: S,
        categories: C,
        payload: Option<TracePayloadT>,
        start_ns: u64,
        duration_ns: u64,
    ) -> Self
    where
        S: Into<StrCow>,
        C: Into<CategoriesT>,
    {
        Self {
            name: name.into(),
            categories: Some(categories.into()),
            timestamp_us: ns_to_us(start_ns),
            event_type: SampleEventType::CompleteDuration,
            duration_us: Some(ns_to_us(duration_ns)),
            tid: sys_tid::current_tid().unwrap(),
            thread_name: Sample::thread_name(),
            pid: sys_pid::current_pid(),
            args: Some(SampleArgs { payload, metadata_name: None, metadata_sort_index: None }),
        }
    }

    /// Constructs an instantaneous sample.
    pub fn new_instant<S, C>(name: S, categories: C, payload: Option<TracePayloadT>) -> Self
    where
        S: Into<StrCow>,
        C: Into<CategoriesT>,
    {
        Self {
            name: name.into(),
            categories: Some(categories.into()),
            timestamp_us: ns_to_us(time::precise_time_ns()),
            event_type: SampleEventType::Instant,
            duration_us: None,
            tid: sys_tid::current_tid().unwrap(),
            thread_name: Sample::thread_name(),
            pid: sys_pid::current_pid(),
            args: Some(SampleArgs { payload, metadata_name: None, metadata_sort_index: None }),
        }
    }

    fn new_metadata(timestamp_ns: u64, meta: MetadataType, tid: u64) -> Self {
        let sample_name = to_cow_str(meta.sample_name());
        let (metadata_name, sort_index) = meta.consume();

        Self {
            name: sample_name,
            categories: None,
            timestamp_us: ns_to_us(timestamp_ns),
            event_type: SampleEventType::Metadata,
            duration_us: None,
            tid,
            thread_name: None,
            pid: sys_pid::current_pid(),
            args: Some(SampleArgs {
                payload: None,
                metadata_name: metadata_name.map(Cow::Owned),
                metadata_sort_index: sort_index,
            }),
        }
    }
}

impl PartialEq for Sample {
    fn eq(&self, other: &Sample) -> bool {
        self.timestamp_us == other.timestamp_us
            && self.name == other.name
            && self.categories == other.categories
            && self.pid == other.pid
            && self.tid == other.tid
            && self.event_type == other.event_type
            && self.args == other.args
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
        self.timestamp_us.cmp(&other.timestamp_us)
    }
}

impl Hash for Sample {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.pid, self.timestamp_us).hash(state);
    }
}

#[must_use]
pub struct SampleGuard<'a> {
    sample: Option<Sample>,
    trace: Option<&'a Trace>,
}

impl<'a> SampleGuard<'a> {
    #[inline]
    pub fn new_disabled() -> Self {
        Self { sample: None, trace: None }
    }

    #[inline]
    fn new<S, C>(trace: &'a Trace, name: S, categories: C, payload: Option<TracePayloadT>) -> Self
    where
        S: Into<StrCow>,
        C: Into<CategoriesT>,
    {
        // TODO(vlovich): optimize this path to use the Complete event type
        // rather than emitting an explicit start/stop to reduce the size of
        // the generated JSON.
        let guard = Self {
            sample: Some(Sample::new_duration_marker(
                name,
                categories,
                payload,
                SampleEventType::DurationBegin,
            )),
            trace: Some(&trace),
        };
        trace.record(guard.sample.as_ref().unwrap().clone());
        guard
    }
}

impl<'a> Drop for SampleGuard<'a> {
    fn drop(&mut self) {
        if let Some(ref mut trace) = self.trace {
            let mut sample = self.sample.take().unwrap();
            sample.timestamp_us = ns_to_us(time::precise_time_ns());
            sample.event_type = SampleEventType::DurationEnd;
            trace.record(sample);
        }
    }
}

/// Returns the file name of the EXE if possible, otherwise the full path, or
/// None if an irrecoverable error occured.
fn exe_name() -> Option<String> {
    match std::env::current_exe() {
        Ok(exe_name) => match exe_name.file_name() {
            Some(filename) => filename.to_str().map(ToString::to_string),
            None => {
                let full_path = exe_name.into_os_string();
                let full_path_str = full_path.into_string();
                match full_path_str {
                    Ok(s) => Some(s),
                    Err(e) => {
                        warn!("Failed to get string representation: {:?}", e);
                        None
                    }
                }
            }
        },
        Err(ref e) => {
            warn!("Failed to get path to current exe: {:?}", e);
            None
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
        Self { enabled: AtomicBool::new(false), samples: Mutex::new(FixedLifoDeque::new()) }
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
    pub(crate) fn record(&self, sample: Sample) {
        let mut all_samples = self.samples.lock().unwrap();
        all_samples.push_back(sample);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(AtomicOrdering::Relaxed)
    }

    pub fn instant<S, C>(&self, name: S, categories: C)
    where
        S: Into<StrCow>,
        C: Into<CategoriesT>,
    {
        if self.is_enabled() {
            self.record(Sample::new_instant(name, categories, None));
        }
    }

    pub fn instant_payload<S, C, P>(&self, name: S, categories: C, payload: P)
    where
        S: Into<StrCow>,
        C: Into<CategoriesT>,
        P: Into<TracePayloadT>,
    {
        if self.is_enabled() {
            self.record(Sample::new_instant(name, categories, Some(payload.into())));
        }
    }

    pub fn block<S, C>(&self, name: S, categories: C) -> SampleGuard
    where
        S: Into<StrCow>,
        C: Into<CategoriesT>,
    {
        if !self.is_enabled() {
            SampleGuard::new_disabled()
        } else {
            SampleGuard::new(&self, name, categories, None)
        }
    }

    pub fn block_payload<S, C, P>(&self, name: S, categories: C, payload: P) -> SampleGuard
    where
        S: Into<StrCow>,
        C: Into<CategoriesT>,
        P: Into<TracePayloadT>,
    {
        if !self.is_enabled() {
            SampleGuard::new_disabled()
        } else {
            SampleGuard::new(&self, name, categories, Some(payload.into()))
        }
    }

    pub fn closure<S, C, F, R>(&self, name: S, categories: C, closure: F) -> R
    where
        S: Into<StrCow>,
        C: Into<CategoriesT>,
        F: FnOnce() -> R,
    {
        // TODO: simplify this through the use of scopeguard crate
        let start = time::precise_time_ns();
        let result = closure();
        let end = time::precise_time_ns();
        if self.is_enabled() {
            self.record(Sample::new_duration(name, categories, None, start, end - start));
        }
        result
    }

    pub fn closure_payload<S, C, P, F, R>(
        &self,
        name: S,
        categories: C,
        closure: F,
        payload: P,
    ) -> R
    where
        S: Into<StrCow>,
        C: Into<CategoriesT>,
        P: Into<TracePayloadT>,
        F: FnOnce() -> R,
    {
        // TODO: simplify this through the use of scopeguard crate
        let start = time::precise_time_ns();
        let result = closure();
        let end = time::precise_time_ns();
        if self.is_enabled() {
            self.record(Sample::new_duration(
                name,
                categories,
                Some(payload.into()),
                start,
                end - start,
            ));
        }
        result
    }

    pub fn samples_cloned_unsorted(&self) -> Vec<Sample> {
        let all_samples = self.samples.lock().unwrap();
        if all_samples.is_empty() {
            return Vec::with_capacity(0);
        }

        let mut as_vec = Vec::with_capacity(all_samples.len() + 10);
        let first_sample_timestamp = all_samples.front().map_or(0, |ref s| s.timestamp_us);
        let tid =
            all_samples.front().map_or_else(|| sys_tid::current_tid().unwrap(), |ref s| s.tid);

        if let Some(exe_name) = exe_name() {
            as_vec.push(Sample::new_metadata(
                first_sample_timestamp,
                MetadataType::ProcessName { name: exe_name },
                tid,
            ));
        }

        let mut thread_names: HashMap<u64, StrCow> = HashMap::new();

        for sample in all_samples.iter() {
            if let Some(ref thread_name) = sample.thread_name {
                let previous_name = thread_names.insert(sample.tid, thread_name.clone());
                if previous_name.is_none() || previous_name.unwrap() != *thread_name {
                    as_vec.push(Sample::new_metadata(
                        first_sample_timestamp,
                        MetadataType::ThreadName { name: thread_name.to_string() },
                        sample.tid,
                    ));
                }
            }
        }

        as_vec.extend(all_samples.iter().cloned());
        as_vec
    }

    #[inline]
    pub fn samples_cloned_sorted(&self) -> Vec<Sample> {
        let mut samples = self.samples_cloned_unsorted();
        samples.sort_unstable();
        samples
    }

    pub fn save<P: AsRef<Path>>(
        &self,
        path: P,
        sort: bool,
    ) -> Result<(), chrome_trace_dump::Error> {
        let traces = if sort { samples_cloned_sorted() } else { samples_cloned_unsorted() };
        let path: &Path = path.as_ref();

        if path.exists() {
            return Err(chrome_trace_dump::Error::already_exists());
        }

        let mut trace_file = fs::File::create(&path)?;

        chrome_trace_dump::serialize(&traces, &mut trace_file)
    }
}

lazy_static! {
    static ref TRACE: Trace = Trace::disabled();
}

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
/// The `json_payload` feature makes this ~1.3-~1.5x slower.
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
pub fn trace<S, C>(name: S, categories: C)
where
    S: Into<StrCow>,
    C: Into<CategoriesT>,
{
    TRACE.instant(name, categories);
}

/// Create an instantaneous sample with a payload.  The type the payload
/// conforms to is currently determined by the feature this library is compiled
/// with.  By default, the type is string-like just like name.  If compiled with
/// the `json_payload` then a `serde_json::Value` is expected and  the library
/// acquires a dependency on the `serde_json` crate.
///
/// # Performance
/// A static string has the lowest overhead as no copies are necessary, roughly
/// equivalent performance to a regular trace.  A string that needs to be copied
/// first can make it ~1.7x slower than a regular trace.
///
/// When compiling with `json_payload`, this is ~2.1x slower than a string that
/// needs to be copied (or ~4.5x slower than a static string)
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
///
/// ```rust,ignore
/// xi_trace::trace_payload("my event", &["rpc", "response"], json!({"key": "value"}));
/// ```
#[inline]
pub fn trace_payload<S, C, P>(name: S, categories: C, payload: P)
where
    S: Into<StrCow>,
    C: Into<CategoriesT>,
    P: Into<TracePayloadT>,
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
pub fn trace_block<'a, S, C>(name: S, categories: C) -> SampleGuard<'a>
where
    S: Into<StrCow>,
    C: Into<CategoriesT>,
{
    TRACE.block(name, categories)
}

/// See `trace_block` for how the block works and `trace_payload` for a
/// discussion on payload.
#[inline]
pub fn trace_block_payload<'a, S, C, P>(name: S, categories: C, payload: P) -> SampleGuard<'a>
where
    S: Into<StrCow>,
    C: Into<CategoriesT>,
    P: Into<TracePayloadT>,
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
pub fn trace_closure<S, C, F, R>(name: S, categories: C, closure: F) -> R
where
    S: Into<StrCow>,
    C: Into<CategoriesT>,
    F: FnOnce() -> R,
{
    TRACE.closure(name, categories, closure)
}

/// See `trace_closure` for how the closure works and `trace_payload` for a
/// discussion on payload.
#[inline]
pub fn trace_closure_payload<S, C, P, F, R>(name: S, categories: C, closure: F, payload: P) -> R
where
    S: Into<StrCow>,
    C: Into<CategoriesT>,
    P: Into<TracePayloadT>,
    F: FnOnce() -> R,
{
    TRACE.closure_payload(name, categories, closure, payload)
}

#[inline]
pub fn samples_len() -> usize {
    TRACE.get_samples_count()
}

/// Returns all the samples collected so far.  There is no guarantee that the
/// samples are ordered chronologically for several reasons:
///
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

/// Save tracing data to to supplied path, using the Trace Viewer format. Trace file can be opened
/// using the Chrome browser by visiting the URL `about:tracing`. If `sorted_chronologically` is
/// true then sort output traces chronologically by each trace's time of creation.
#[inline]
pub fn save<P: AsRef<Path>>(path: P, sort: bool) -> Result<(), chrome_trace_dump::Error> {
    TRACE.save(path, sort)
}

#[cfg(test)]
#[rustfmt::skip]
mod tests {
    use super::*;
    #[cfg(feature = "benchmarks")]
    use test::Bencher;
    #[cfg(feature = "benchmarks")]
    use test::black_box;

    #[cfg(not(feature = "json_payload"))]
    fn to_payload(value: &'static str) -> &'static str {
        value
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
        // 1 for exe name & 1 for the thread name
        assert_eq!(trace.samples_cloned_unsorted().len(), 7);
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
        // +2 for exe & thread name.
        assert_eq!(trace.samples_cloned_unsorted().len(), 3);

        trace.closure_payload("y", &["test"], || {},
                              to_payload("test_get_samples"));
        assert_eq!(trace.samples_cloned_unsorted().len(), 4);

        trace.closure_payload("z", &["test"], || {},
                              to_payload("test_get_samples"));

        let snapshot = trace.samples_cloned_unsorted();
        assert_eq!(snapshot.len(), 5);

        assert_eq!(snapshot[0].name, "process_name");
        assert_eq!(snapshot[0].args.as_ref().unwrap().metadata_name.as_ref().is_some(), true);
        assert_eq!(snapshot[1].name, "thread_name");
        assert_eq!(snapshot[1].args.as_ref().unwrap().metadata_name.as_ref().is_some(), true);
        assert_eq!(snapshot[2].name, "x");
        assert_eq!(snapshot[3].name, "y");
        assert_eq!(snapshot[4].name, "z");
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
            let _ = trace.block_payload("z", &["test"], to_payload("test_get_samples_nested_trace"));
            trace.instant_payload("c", &["test"], to_payload("test_get_samples_nested_trace"));
        }, to_payload("test_get_samples_nested_trace"));

        let snapshot = trace.samples_cloned_unsorted();
        // +2 for exe & thread name
        assert_eq!(snapshot.len(), 9);

        assert_eq!(snapshot[0].name, "process_name");
        assert_eq!(snapshot[0].args.as_ref().unwrap().metadata_name.as_ref().is_some(), true);
        assert_eq!(snapshot[1].name, "thread_name");
        assert_eq!(snapshot[1].args.as_ref().unwrap().metadata_name.as_ref().is_some(), true);
        assert_eq!(snapshot[2].name, "a");
        assert_eq!(snapshot[3].name, "b");
        assert_eq!(snapshot[4].name, "y");
        assert_eq!(snapshot[5].name, "z");
        assert_eq!(snapshot[6].name, "z");
        assert_eq!(snapshot[7].name, "c");
        assert_eq!(snapshot[8].name, "x");
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

        // NOTE: 1 us sleeps are inserted as the first line of a closure to
        // ensure that when the samples are sorted by time they come out in a
        // stable order since the resolution of timestamps is 1us.
        // NOTE 2: from_micros is currently in unstable so using new
        trace.closure_payload("x", &["test"], || {
            std::thread::sleep(std::time::Duration::new(0, 1000));
            trace.instant_payload("a", &["test"], to_payload("test_get_sorted_samples"));
            trace.closure_payload("y", &["test"], || {
                std::thread::sleep(std::time::Duration::new(0, 1000));
                trace.instant_payload("b", &["test"], to_payload("test_get_sorted_samples"));
            }, to_payload("test_get_sorted_samples"));
            let _ = trace.block_payload("z", &["test"], to_payload("test_get_sorted_samples"));
            trace.instant("c", &["test"]);
        }, to_payload("test_get_sorted_samples"));

        let snapshot = trace.samples_cloned_sorted();
        // +2 for exe & thread name.
        assert_eq!(snapshot.len(), 9);

        assert_eq!(snapshot[0].name, "process_name");
        assert_eq!(snapshot[0].args.as_ref().unwrap().metadata_name.as_ref().is_some(), true);
        assert_eq!(snapshot[1].name, "thread_name");
        assert_eq!(snapshot[1].args.as_ref().unwrap().metadata_name.as_ref().is_some(), true);
        assert_eq!(snapshot[2].name, "x");
        assert_eq!(snapshot[3].name, "a");
        assert_eq!(snapshot[4].name, "y");
        assert_eq!(snapshot[5].name, "b");
        assert_eq!(snapshot[6].name, "z");
        assert_eq!(snapshot[7].name, "z");
        assert_eq!(snapshot[8].name, "c");
    }

    #[test]
    fn test_cross_process_samples() {
        let mut samples = vec![
            Sample::new_instant("local pid", &[], None),
            Sample::new_instant("remote pid", &[], None)];
        samples[0].pid = 1;
        samples[0].timestamp_us = 10;

        samples[1].pid = 2;
        samples[1].timestamp_us = 5;

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
            black_box(|| {
                let _ = trace.block_payload(
                    "something", &["benchmark"],
                    to_payload("some payload for the block"));
            });
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
                    to_payload("some description of the closure"))));
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
