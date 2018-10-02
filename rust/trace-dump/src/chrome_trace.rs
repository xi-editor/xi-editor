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

use xi_trace::{Sample, SampleType, StrCow, TracePayloadT};
use serde::Deserialize;
use serde_json;
use std::io::{Error as IOError, Read, Write};
use std::iter::Iterator;

pub enum OutputFormat {
    /// Output the samples as a JSON array.  Each array entry is an object
    /// describing the sample.
    JsonArray,
}

#[derive(Debug)]
pub enum Error {
    Io(IOError),
    Json(serde_json::Error),
    DecodingFormat(String)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ChromeTraceArrayEntryArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    xi_sample_id: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<TracePayloadT>,
}

/// This is the struct that represents a single entry within an array of
/// samples.  Vec<ChromeTraceArrayEntry> would represent a single complete
/// trace.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct ChromeTraceArrayEntry {
    name: StrCow,
    cat: StrCow,
    ph: StrCow,
    #[serde(rename = "ts")]
    ts_us: u64,
    pid: u64,
    tid: u64,
    args: Option<ChromeTraceArrayEntryArgs>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum ChromeTraceArrayEntries {
    Array(Vec<ChromeTraceArrayEntry>)
}

fn to_us(ns: u64) -> u64 {
    ns / 1000
}

fn to_ns(us: u64) -> u64 {
    us * 1000
}

fn event_type(sample: &Sample, begin: bool) -> &'static str {
    match sample.sample_type {
        SampleType::Instant => "i",
        SampleType::Duration => if begin { "B" } else { "E" },
    }
}

fn sample_to_entry(sample: &Sample, begin: bool) -> ChromeTraceArrayEntry {
    ChromeTraceArrayEntry {
        name: sample.name.clone(),
        cat: StrCow::from(sample.categories.join(",")),
        ph: StrCow::from(event_type(sample, begin)),
        ts_us: to_us(if begin { sample.start_ns } else { sample.get_end_ns() }),
        pid: sample.pid,
        tid: sample.tid,
        args: Some(ChromeTraceArrayEntryArgs {
            xi_sample_id: Some(sample.sample_id),
            payload: sample.payload.clone(),
        })
    }
}

impl<'a, A> From<A> for ChromeTraceArrayEntries where A: IntoIterator<Item = &'a Sample> {
    fn from(samples: A) -> Self {
        // Worst case is every sample is a duration.  if we were to use an
        // original capacity of samples.len() then we'd inevitably encounter
        // at least 1 duration & thus the underlying Vec would double the
        // capacity anyway.  Alternatively, we could scan the vector to count
        // the number of durations & add that to samples.len() for the exact
        // final capacity.
        let samples_iter = samples.into_iter();
        let mut result = Vec::with_capacity(samples_iter.size_hint().0 * 2);

        // Instantaneous samples have a 1:1 mapping to chrome trace event
        // samples.  Duration samples output 2 chrome trace events (one for the
        // start and one for the end).
        for sample in samples_iter {
            match sample.sample_type {
                SampleType::Instant => result.push(sample_to_entry(&sample, true)),
                SampleType::Duration => {
                    result.push(sample_to_entry(&sample, true));
                    result.push(sample_to_entry(&sample, false));
                }
            }
        }
        ChromeTraceArrayEntries::Array(result)
    }
}

// temporary while TryFrom is still nightly.
pub trait XiTryFrom<T>: Sized {
    type Error;
    fn try_from(value: T) -> Result<Self, Self::Error>;
}

fn try_from(trace_entry: ChromeTraceArrayEntry, default_sample_id: usize)
    -> Result<Sample, Error> {
    // Chrome trace stores the categories as comma-separated.
    // Split it back out into a vector.
    let categories = trace_entry.cat.split(',').map(
        |s| s.to_string()).collect::<Vec<String>>();

    let (sample_id, payload ) = trace_entry.args.map_or((default_sample_id, None), |args| {
        (args.xi_sample_id.unwrap_or(default_sample_id), args.payload)
    });

    let ph = trace_entry.ph.as_ref();

    let mut converted = match ph {
        "i" => Ok(Sample::new_instant(trace_entry.name, categories, payload)),
        "B" => Ok(Sample::new(trace_entry.name, categories, payload)),
        _ => Err(Error::DecodingFormat(
                format!("Entry has unexpected ph value {}",
                        ph)))
    }?;

    converted.sample_id = sample_id;
    converted.pid = trace_entry.pid;
    converted.tid = trace_entry.tid;
    converted.start_ns = to_ns(trace_entry.ts_us);
    Ok(converted)
}

impl XiTryFrom<Vec<ChromeTraceArrayEntry>> for Vec<Sample> {
    type Error = Error;

    /// Converting Chrome trace event back into a Sample is a bit more work
    /// because a Sample can represent a duration so we have to merge Chrome
    /// trace events that indicate start/end.
    fn try_from(trace_entries: Vec<ChromeTraceArrayEntry>) -> Result<Self, Error> {
        let mut result = Vec::with_capacity(trace_entries.len());
        for trace_entry in trace_entries {
            if trace_entry.ph == "E" {
                // Got an end of a duration measure so look back into the
                // samples we've already converted to populate the end
                // timestamp.
                let mut found = false;
                for mut existing_sample in result.iter_mut().rev() {
                    if is_begin_sample(&existing_sample, trace_entry.pid, trace_entry.tid, &trace_entry.name) {
                        existing_sample.set_end_ns(to_ns(trace_entry.ts_us));
                        found = true;
                        break;
                    }
                }
                if !found {
                    return Err(Error::DecodingFormat(
                        format!("Entry {:?} found but no preceding start sample exists",
                                trace_entry)));
                }
            } else {
                let default_sample_id = result.len();
                result.push(try_from(trace_entry, default_sample_id)?);
            }
        }
        Ok(result)
    }
}

impl XiTryFrom<ChromeTraceArrayEntries> for Vec<Sample> {
    type Error = Error;

    /// Converting Chrome trace event back into a Sample is a bit more work
    /// because a Sample can represent a duration so we have to merge Chrome
    /// trace events that indicate start/end.
    fn try_from(entries: ChromeTraceArrayEntries) -> Result<Self, Error> {
        match entries {
            ChromeTraceArrayEntries::Array(samples_array) => Vec::try_from(samples_array),
        }
    }
}

/// This serializes the samples into the [Chrome trace event format](https://www.google.com/url?sa=t&rct=j&q=&esrc=s&source=web&cd=1&ved=0ahUKEwiJlZmDguXYAhUD4GMKHVmEDqIQFggpMAA&url=https%3A%2F%2Fdocs.google.com%2Fdocument%2Fd%2F1CvAClvFfyA5R-PhYUmn5OOQtYMH4h6I0nSsKchNAySU%2Fpreview&usg=AOvVaw0tBFlVbDVBikdzLqgrWK3g).
///
/// # Arguments
/// `samples` - Something that can be converted into an iterator of sample
/// references.
/// `format` - Which trace format to save the data in.  There are four total
/// formats described in the document.
/// `output` - Where to write the serialized result.
///
/// # Returns
/// A `Result<(), Error>` that indicates if serialization was successful or the
/// details of any error that occured.
///
/// # Examples
/// ```norun
/// let samples = xi_trace::samples_cloned_sorted();
/// let mut serialized = Vec::<u8>::new();
/// chrome_trace::serialize(samples.iter(), OutputFormat::JsonArray, serialized);
/// ```
pub fn serialize<'a, I, W>(samples: I, _format: OutputFormat, output: W)
    -> Result<(), Error> 
    where I: IntoIterator<Item = &'a Sample>, W: Write
{
    let converted = ChromeTraceArrayEntries::from(samples.into_iter());
    serde_json::to_writer(output, &converted).map_err(Error::Json)
}

pub fn to_value(samples: &[Sample], format: OutputFormat)
    -> Result<serde_json::Value, Error>
{
    match format {
        OutputFormat::JsonArray => serde_json::to_value(samples).map_err(Error::Json)
    }
}

fn is_begin_sample(sample: &Sample, pid: u64, tid: u64, name: &str) -> bool {
    if sample.sample_type != SampleType::Duration {
        false
    } else if sample.pid != pid || sample.tid != tid {
        false
    } else if sample.name != name {
        false
    } else {
        true
    }
}

pub fn decode(samples: &serde_json::Value) -> Result<Vec<Sample>, Error> {
    let entries = ChromeTraceArrayEntries::deserialize(samples).map_err(Error::Json)?;
    Vec::try_from(entries)
}

pub fn deserialize<R>(input: R) -> Result<Vec<Sample>, Error>
    where R: Read
{
    let entries : ChromeTraceArrayEntries = serde_json::from_reader(input).map_err(Error::Json)?;
    Vec::try_from(entries)
}
