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
#![allow(
    clippy::if_same_then_else,
    clippy::needless_bool,
    clippy::needless_pass_by_value,
    clippy::ptr_arg
)]

#[cfg(all(test, feature = "benchmarks"))]
extern crate test;

use super::Sample;
use serde_json;
use std::fs::File;
use std::io::{Error as IOError, Read, Write};

#[derive(Debug)]
pub enum Error {
    Io(IOError),
    Json(serde_json::Error),
    DecodingFormat(String),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum ChromeTraceArrayEntries {
    Array(Vec<Sample>),
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
/// serialize(samples.iter(), serialized);
/// ```
pub fn serialize<W>(samples: &Vec<Sample>, output: W) -> Result<(), Error>
where
    W: Write,
{
    serde_json::to_writer(output, samples).map_err(Error::Json)
}

pub fn to_value(samples: &Vec<Sample>) -> Result<serde_json::Value, Error> {
    serde_json::to_value(samples).map_err(Error::Json)
}

pub fn decode(samples: serde_json::Value) -> Result<Vec<Sample>, Error> {
    serde_json::from_value(samples).map_err(Error::Json)
}

pub fn deserialize<R>(input: R) -> Result<Vec<Sample>, Error>
where
    R: Read,
{
    serde_json::from_reader(input).map_err(Error::Json)
}

/// Save tracing data to path pointed to by the environment variable TRACE_OUTPUT, using the Trace
/// Viewer format. Save path defaults to `./target/trace_output.trace`. Trace file can be opened
/// with the Chrome browser by visiting the URL `about:tracing`. If `sorted_chronologically` is true
/// then sort output traces chronologically by time of creation.
pub fn save_traces(sorted_chronologically: bool) {
    use std::env;

    let traces = if sorted_chronologically {
        super::samples_cloned_sorted()
    } else {
        super::samples_cloned_unsorted()
    };

    let trace_output_path = match env::var("TRACE_OUTPUT") {
        Ok(output_path) => output_path,
        Err(_) => {
            println!("Environment variable TRACE_OUTPUT not set, defaulting to ./target/trace_output.trace");
            String::from("./target/trace_output.trace")
        }
    };

    let mut trace_file = match File::create(&trace_output_path) {
        Ok(f) => f,
        Err(_) => {
            println!("Could not create trace output file at: {}.", &trace_output_path);
            return;
        }
    };

    if serialize(&traces, &mut trace_file).is_err() {
        println!("Could not save trace file at: {}.", &trace_output_path);
    } else {
        println!("Saved trace file at: {}", &trace_output_path);
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "dict_payload")]
    use super::super::{StrCow, TracePayloadT};
    use super::*;
    #[cfg(feature = "benchmarks")]
    use test::Bencher;

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
        json!({ "test": value })
    }

    #[cfg(feature = "chrome_trace_event")]
    #[test]
    fn test_chrome_trace_serialization() {
        use super::super::*;

        let trace = Trace::enabled(Config::with_limit_count(10));
        trace.instant("sample1", &["test", "chrome"]);
        trace.instant_payload("sample2", &["test", "chrome"], to_payload("payload 2"));
        trace.instant_payload("sample3", &["test", "chrome"], to_payload("payload 3"));
        trace.closure_payload(
            "sample4",
            &["test", "chrome"],
            || {
                let _guard = trace.block("sample5", &["test,chrome"]);
            },
            to_payload("payload 4"),
        );

        let samples = trace.samples_cloned_unsorted();

        let mut serialized = Vec::<u8>::new();

        let result = serialize(&samples, &mut serialized);
        assert!(result.is_ok(), "{:?}", result);

        let decoded_result: Vec<serde_json::Value> = serde_json::from_slice(&serialized).unwrap();
        assert_eq!(decoded_result.len(), 8);
        assert_eq!(decoded_result[0]["name"].as_str().unwrap(), "process_name");
        assert_eq!(decoded_result[1]["name"].as_str().unwrap(), "thread_name");
        for i in 2..5 {
            assert_eq!(decoded_result[i]["name"].as_str().unwrap(), samples[i].name);
            assert_eq!(decoded_result[i]["cat"].as_str().unwrap(), "test,chrome");
            assert_eq!(decoded_result[i]["ph"].as_str().unwrap(), "i");
            assert_eq!(decoded_result[i]["ts"], samples[i].timestamp_us);
            let nth_sample = &samples[i];
            let nth_args = nth_sample.args.as_ref().unwrap();
            assert_eq!(decoded_result[i]["args"]["xi_payload"], json!(nth_args.payload.as_ref()));
        }
        assert_eq!(decoded_result[5]["ph"], "B");
        assert_eq!(decoded_result[6]["ph"], "E");
        assert_eq!(decoded_result[7]["ph"], "X");
    }

    #[cfg(feature = "chrome_trace_event")]
    #[test]
    fn test_chrome_trace_deserialization() {
        use super::super::*;

        let trace = Trace::enabled(Config::with_limit_count(10));
        trace.instant("sample1", &["test", "chrome"]);
        trace.instant_payload("sample2", &["test", "chrome"], to_payload("payload 2"));
        trace.instant_payload("sample3", &["test", "chrome"], to_payload("payload 3"));
        trace.closure_payload("sample4", &["test", "chrome"], || (), to_payload("payload 4"));

        let samples = trace.samples_cloned_unsorted();

        let mut serialized = Vec::<u8>::new();
        let result = serialize(&samples, &mut serialized);
        assert!(result.is_ok(), "{:?}", result);

        let deserialized_samples = deserialize(serialized.as_slice()).unwrap();
        assert_eq!(deserialized_samples, samples);
    }

    #[cfg(all(feature = "chrome_trace_event", feature = "benchmarks"))]
    #[bench]
    fn bench_chrome_trace_serialization_one_element(b: &mut Bencher) {
        use super::chrome_trace_dump::*;

        let mut serialized = Vec::<u8>::new();
        let samples = [super::Sample::new_instant("trace1", &["benchmark", "test"], None)];
        b.iter(|| {
            serialized.clear();
            serialize(samples.iter(), &mut serialized).unwrap();
        });
    }

    #[cfg(all(feature = "chrome_trace_event", feature = "benchmarks"))]
    #[bench]
    fn bench_chrome_trace_serialization_multiple_elements(b: &mut Bencher) {
        use super::super::*;
        use super::chrome_trace_dump::*;

        let mut serialized = Vec::<u8>::new();
        let mut samples = [
            Sample::new_instant("trace1", &["benchmark", "test"], None),
            Sample::new_instant("trace2", &["benchmark"], None),
            Sample::new_duration("trace3", &["benchmark"], Some(to_payload("some payload"), 0)),
            Sample::new_instant("trace4", &["benchmark"], None),
        ];

        b.iter(|| {
            serialized.clear();
            serialize(samples.iter(), &mut serialized).unwrap();
        });
    }
}
