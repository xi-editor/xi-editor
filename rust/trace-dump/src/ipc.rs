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

use std::io::Error as IOError;
use std::io::{Read, Write};
use xi_trace::*;

use bincode;

#[derive(Debug)]
pub enum Error {
    IOError(IOError),
    BincodeError(bincode::Error)
}

pub fn serialize_to_bytes<'a>(samples: &Vec<Sample>) -> Result<Vec<u8>, Error>
{
    bincode::serialize(&samples, bincode::Infinite)
        .map_err(|e| Error::BincodeError(e))
}

pub fn serialize_to_stream<'a, W>(samples: &Vec<Sample>, output: &mut W)
    -> Result<(), Error>
    where W: Write
{
    bincode::serialize_into(output, &samples, bincode::Infinite)
        .map_err(|e| Error::BincodeError(e))
}

pub fn serialized_size(samples: &Vec<Sample>) -> u64 {
    bincode::serialized_size(samples)
}

pub fn deserialize_from_bytes(encoded: &[u8]) -> Result<Vec<Sample>, Error> {
    bincode::deserialize(encoded)
        .map_err(|e| Error::BincodeError(e))
}

pub fn deserialize<R>(reader: &mut R) -> Result<Vec<Sample>, Error>
    where R: Read
{
    bincode::deserialize_from(reader, bincode::Infinite)
        .map_err(|e| Error::BincodeError(e))
}
