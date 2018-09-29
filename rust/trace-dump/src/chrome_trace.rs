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

use serde_json;
use std::io::{Error as IOError, Read, Write};
use xi_trace::Sample;

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
/// chrome_trace::serialize(samples.iter(), serialized);
/// ```
pub fn serialize<'a, W>(samples: &Vec<Sample>, output: W) -> Result<(), Error>
where
    W: Write,
{
    serde_json::to_writer(output, samples).map_err(|e| Error::Json(e))
}

pub fn to_value(samples: &Vec<Sample>) -> Result<serde_json::Value, Error> {
    serde_json::to_value(samples).map_err(|e| Error::Json(e))
}

pub fn decode(samples: serde_json::Value) -> Result<Vec<Sample>, Error> {
    serde_json::from_value(samples).map_err(|e| Error::Json(e))
}

pub fn deserialize<R>(input: R) -> Result<Vec<Sample>, Error>
where
    R: Read,
{
    serde_json::from_reader(input).map_err(|e| Error::Json(e))
}
