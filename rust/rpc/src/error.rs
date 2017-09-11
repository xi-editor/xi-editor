// Copyright 2017 Google Inc. All rights reserved.
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

use std::io;
use std::fmt;

use serde_json::{Value, Error as JsonError};
use serde::de::{Deserializer, Deserialize};
use serde::ser::{Serializer, Serialize};

/// Errors that can occur when sending an RPC.
#[derive(Debug)]
pub enum Error {
    /// An IO error occurred on the underlying communication channel.
    IoError(io::Error),
    /// The peer returned an error.
    RemoteError(RemoteError),
    /// The peer closed its connection.
    PeerDisconnect,
    /// The peer sent a response containing the id, but was malformed.
    InvalidResponse,
}

/// Errors that can occur when attemping to read a message.
#[derive(Debug)]
pub enum ReadError {
    /// An error occured in the underlying stream
    Io(io::Error),
    /// The message was not valid JSON.
    Json(JsonError),
    /// The message was not a JSON object.
    NotObject,
    /// The peer closed the connection.
    Disconnect,
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ReadError::Io(ref err) => write!(f, "I/O Error: {:?}", err),
            ReadError::Json(ref err) => write!(f, "JSON Error: {:?}", err),
            ReadError::NotObject => write!(f, "JSON message was not an object."),
            ReadError::Disconnect => write!(f, "Peer closd the connection."),
        }
    }
}

impl From<JsonError> for ReadError {
    fn from(err: JsonError) -> ReadError {
        ReadError::Json(err)
    }
}

impl From<io::Error> for ReadError {
    fn from(err: io::Error) -> ReadError {
        ReadError::Io(err)
    }
}

//TODO: code review discussion: do we want, in general, to support these
//parsing-related error codes (borrowed from the JSON-RPC spec) or do
//we prefer to crash?
/// Errors that can occur in the process of receiving an RPC.
///
/// These errors are based off the errors defined in the JSON-RPC spec,
/// and are intended to go over the wire. Serialized, they are an
/// object with three fields: 'code', 'message', and optionally 'data'.
///
/// The first four members represent message parsing errors.
/// The last member represents application logic errors.
#[derive(Debug, Clone, PartialEq)]
pub enum RemoteError {
    /// The JSON was valid, but was not a correctly formed request.
    InvalidRequest(Option<Value>),
    /// The called method is not handled.
    MethodNotFound(Option<Value>),
    /// The params were not valid for the method.
    InvalidParams(Option<Value>),
    /// The message could not be parsed.
    ///
    /// This is a catch-all. Where possible, use a more specific error.
    Parse(Option<Value>),
    /// A custom error.
    Custom { code: i64, message: String, data: Option<Value> },
    /// An error was received, but it was not a recognizeable Error object
    Unknown(Value),
}

impl RemoteError {
    /// Creates a new custom error.
    pub fn custom<S, V>(code: i64, message: S, data: V) -> Self
        where S: AsRef<str>,
              V: Into<Option<Value>>,
    {
        let message = message.as_ref().into();
        let data = data.into();
        RemoteError::Custom { code, message, data }
    }
}

impl From<JsonError> for RemoteError {
    fn from(err: JsonError) -> RemoteError {
        RemoteError::Parse(Some(json!(err.to_string())))
    }
}

impl From<RemoteError> for Error {
    fn from(err: RemoteError) -> Error {
        Error::RemoteError(err)
    }
}

#[derive(Deserialize, Serialize)]
struct ErrorHelper {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl<'de> Deserialize<'de> for RemoteError
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where D: Deserializer<'de>
    {
        let v = Value::deserialize(deserializer)?;
        let resp = match ErrorHelper::deserialize(&v) {
            Ok(resp) => resp,
            Err(_) => return Ok(RemoteError::Unknown(v)),
        };

        Ok(match resp.code {
            -32700 => RemoteError::Parse(resp.data),
            -32600 => RemoteError::InvalidRequest(resp.data),
            -32601 => RemoteError::MethodNotFound(resp.data),
            -32602 => RemoteError::InvalidParams(resp.data),
            _ => RemoteError::Custom { code: resp.code, message: resp.message, data: resp.data },
        })
    }
}

impl Serialize for RemoteError
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where S: Serializer
    {
        let (code, message, data) = match *self {
             RemoteError::Parse(ref d) => (-32700, "Parse error", d),
             RemoteError::InvalidRequest(ref d) => (-32600, "Invalid request", d),
             RemoteError::MethodNotFound(ref d) => (-32601, "Method not found", d),
             RemoteError::InvalidParams(ref d) => (-32602, "Invalid params", d),
             RemoteError::Custom { code, ref message, ref data } => {
                 (code, message.as_ref(), data)
             }
             RemoteError::Unknown(_) => panic!("The 'Unknown' error variant is\
                                               not intended for client use."),
        };
        let message = message.to_owned();
        let data = data.to_owned();
        let err = ErrorHelper { code, message, data };
        err.serialize(serializer)
    }
}

