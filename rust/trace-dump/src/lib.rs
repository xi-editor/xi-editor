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
#![cfg_attr(
    feature = "cargo-clippy",
    allow(if_same_then_else, needless_bool, needless_pass_by_value, ptr_arg)
)]

extern crate xi_trace;

#[cfg(any(feature = "chrome_trace_event"))]
extern crate serde;

#[macro_use]
extern crate serde_derive;

#[cfg(all(not(test), feature = "chrome_trace_event"))]
extern crate serde_json;

#[cfg(test)]
#[macro_use]
extern crate serde_json;

#[cfg(all(test, feature = "benchmarks"))]
extern crate test;

#[cfg(feature = "chrome_trace_event")]
pub mod chrome_trace;

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "benchmarks")]
    use test::Bencher;
    #[cfg(feature = "dict_payload")]
    use xi_trace::{StrCow, TracePayloadT};

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
        use xi_trace::*;

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

        let result = chrome_trace::serialize(&samples, &mut serialized);
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
            let mut nth_args = nth_sample.args.as_ref().unwrap();
            assert_eq!(decoded_result[i]["args"]["xi_payload"], json!(nth_args.payload.as_ref()));
        }
        assert_eq!(decoded_result[5]["ph"], "B");
        assert_eq!(decoded_result[6]["ph"], "E");
        assert_eq!(decoded_result[7]["ph"], "X");
    }

    #[cfg(feature = "chrome_trace_event")]
    #[test]
    fn test_chrome_trace_deserialization() {
        use xi_trace::*;

        let trace = Trace::enabled(Config::with_limit_count(10));
        trace.instant("sample1", &["test", "chrome"]);
        trace.instant_payload("sample2", &["test", "chrome"], to_payload("payload 2"));
        trace.instant_payload("sample3", &["test", "chrome"], to_payload("payload 3"));
        trace.closure_payload("sample4", &["test", "chrome"], || (), to_payload("payload 4"));

        let samples = trace.samples_cloned_unsorted();

        let mut serialized = Vec::<u8>::new();
        let result = chrome_trace::serialize(&samples, &mut serialized);
        assert!(result.is_ok(), "{:?}", result);

        let deserialized_samples = chrome_trace::deserialize(serialized.as_slice()).unwrap();
        assert_eq!(deserialized_samples, samples);
    }

    #[cfg(all(feature = "chrome_trace_event", feature = "benchmarks"))]
    #[bench]
    fn bench_chrome_trace_serialization_one_element(b: &mut Bencher) {
        use chrome_trace::*;

        let mut serialized = Vec::<u8>::new();
        let samples = [xi_trace::Sample::new_instant("trace1", &["benchmark", "test"], None)];
        b.iter(|| {
            serialized.clear();
            serialize(samples.iter(), &mut serialized).unwrap();
        });
    }

    #[cfg(all(feature = "chrome_trace_event", feature = "benchmarks"))]
    #[bench]
    fn bench_chrome_trace_serialization_multiple_elements(b: &mut Bencher) {
        use chrome_trace::*;
        use xi_trace::*;

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
