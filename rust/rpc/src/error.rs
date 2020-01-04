// Copyright 2017 The xi-editor Authors.
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

use std::fmt;
use std::io;

use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};
use serde_json::{Error as JsonError, Value};

/// The possible error outcomes when attempting to send a message.
#[derive(Debug)]
pub enum Error {
    /// An IO error occurred on the underlying communication channel.
    Io(io::Error),
    /// The peer returned an error.
    RemoteError(RemoteError),
    /// The peer closed the connection.
    PeerDisconnect,
    /// The peer sent a response containing the id, but was malformed.
    InvalidResponse,
}

/// The possible error outcomes when attempting to read a message.
#[derive(Debug)]
pub enum ReadError {
    /// An error occurred in the underlying stream
    Io(io::Error),
    /// The message was not valid JSON.
    Json(JsonError),
    /// The message was not a JSON object.
    NotObject,
    /// The the method and params were not recognized by the handler.
    UnknownRequest(JsonError),
    /// The peer closed the connection.
    Disconnect,
}

/// Errors that can be received from the other side of the RPC channel.
///
/// This type is intended to go over the wire. And by convention
/// should `Serialize` as a JSON object with "code", "message",
/// and optionally "data" fields.
///
/// The xi RPC protocol defines one error: `RemoteError::InvalidRequest`,
/// represented by error code `-32600`; however codes in the range
/// `-32700 ... -32000` (inclusive) are reserved for compatability with
/// the JSON-RPC spec.
///
/// # Examples
///
/// An invalid request:
///
/// ```
/// # extern crate xi_rpc;
/// # extern crate serde_json;
/// use xi_rpc::RemoteError;
/// use serde_json::Value;
///
/// let json = r#"{
///     "code": -32600,
///     "message": "Invalid request",
///     "data": "Additional details"
///     }"#;
///
/// let err = serde_json::from_str::<RemoteError>(&json).unwrap();
/// assert_eq!(err,
///            RemoteError::InvalidRequest(
///                Some(Value::String("Additional details".into()))));
/// ```
///
/// A custom error:
///
/// ```
/// # extern crate xi_rpc;
/// # extern crate serde_json;
/// use xi_rpc::RemoteError;
/// use serde_json::Value;
///
/// let json = r#"{
///     "code": 404,
///     "message": "Not Found"
///     }"#;
///
/// let err = serde_json::from_str::<RemoteError>(&json).unwrap();
/// assert_eq!(err, RemoteError::custom(404, "Not Found", None));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum RemoteError {
    /// The JSON was valid, but was not a correctly formed request.
    ///
    /// This Error is used internally, and should not be returned by
    /// clients.
    InvalidRequest(Option<Value>),
    /// A custom error, defined by the client.
    Custom { code: i64, message: String, data: Option<Value> },
    /// An error that cannot be represented by an error object.
    ///
    /// This error is intended to accommodate clients that return arbitrary
    /// error values. It should not be used for new errors.
    Unknown(Value),
}

impl RemoteError {
    /// Creates a new custom error.
    pub fn custom<S, V>(code: i64, message: S, data: V) -> Self
    where
        S: AsRef<str>,
        V: Into<Option<Value>>,
    {
        let message = message.as_ref().into();
        let data = data.into();
        RemoteError::Custom { code, message, data }
    }
}

impl ReadError {
    /// Returns `true` iff this is the `ReadError::Disconnect` variant.
    pub fn is_disconnect(&self) -> bool {
        match *self {
            ReadError::Disconnect => true,
            _ => false,
        }
    }
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ReadError::Io(ref err) => write!(f, "I/O Error: {:?}", err),
            ReadError::Json(ref err) => write!(f, "JSON Error: {:?}", err),
            ReadError::NotObject => write!(f, "JSON message was not an object."),
            ReadError::UnknownRequest(ref err) => write!(f, "Unknown request: {:?}", err),
            ReadError::Disconnect => write!(f, "Peer closed the connection."),
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

impl From<JsonError> for RemoteError {
    fn from(err: JsonError) -> RemoteError {
        RemoteError::InvalidRequest(Some(json!(err.to_string())))
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

impl<'de> Deserialize<'de> for RemoteError {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = Value::deserialize(deserializer)?;
        let resp = match ErrorHelper::deserialize(&v) {
            Ok(resp) => resp,
            Err(_) => return Ok(RemoteError::Unknown(v)),
        };

        Ok(match resp.code {
            -32600 => RemoteError::InvalidRequest(resp.data),
            _ => RemoteError::Custom { code: resp.code, message: resp.message, data: resp.data },
        })
    }
}

impl Serialize for RemoteError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let (code, message, data) = match *self {
            RemoteError::InvalidRequest(ref d) => (-32600, "Invalid request", d),
            RemoteError::Custom { code, ref message, ref data } => (code, message.as_ref(), data),
            RemoteError::Unknown(_) => panic!(
                "The 'Unknown' error variant is \
                 not intended for client use."
            ),
        };
        let message = message.to_owned();
        let data = data.to_owned();
        let err = ErrorHelper { code, message, data };
        err.serialize(serializer)
    }
}
