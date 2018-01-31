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

use xi_trace::{Sample, SampleType};
use serde;
use serde::ser::Serializer;
use serde::ser::SerializeSeq;
use serde_json;
use std::io::{Error as IOError, Write};
use std::iter::Iterator;

pub enum OutputFormat {
    /// Output the samples as a JSON array.  Each array entry is an object
    /// describing the sample.
    JsonArray,
}

#[derive(Debug)]
pub enum Error {
    Io(IOError),
    Json(serde_json::Error)
}

fn to_us(ns: u64) -> u64 {
    ns / 1000
}

fn event_type(sample: &Sample, begin: bool) -> &'static str {
    match sample.sample_type {
        SampleType::Instant => "i",
        SampleType::Duration => if begin { "B" } else { "E" },
    }
}

fn sample_to_json(sample: &Sample, begin: bool) -> serde_json::Value {
    json!({
        "cat": sample.categories.join(","),
        "name": sample.name, 
        "ph": event_type(sample, begin),
        "ts": to_us(if begin { sample.start_ns } else { sample.get_end_ns() }),
        "pid": sample.pid,
        "tid": sample.tid,
        "args": {
            "payload": sample.payload
        }
    })
}

fn serialize_to_json_array<'a, I, W>(samples: I, output: W)
    -> Result<(), Error>
    where I: IntoIterator<Item = &'a Sample>, W: Write
{
    let mut serializer = serde_json::ser::Serializer::new(output);

    // Step 1: Create a serializor for the samples
    serializer.serialize_seq(Some(1))
        .and_then(|mut seq| {
            // Write out each sample...
            samples.into_iter().map(|sample: &Sample| {
                seq.serialize_element(&sample_to_json(&sample, true))
                    // + if it's a range write 2 samples
                    .and_then(|_| {
                        match sample.sample_type {
                            SampleType::Instant => serde::export::Ok(()),
                            SampleType::Duration => seq.serialize_element(&sample_to_json(&sample, false)),
                        }
                    })
            // Reduce all the results from each individual serialization
            }).fold(Ok(()), |acc_res, res| acc_res.and(res))
            // Terminate the serialization
            .and_then(|_| seq.end())
        }).map_err(|err| Error::Json(err))
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
pub fn serialize<'a, I, W>(samples: I, format: OutputFormat, output: W)
    -> Result<(), Error> 
    where I: IntoIterator<Item = &'a Sample>, W: Write
{
    match format {
        OutputFormat::JsonArray => serialize_to_json_array(samples, output)
    }
}
