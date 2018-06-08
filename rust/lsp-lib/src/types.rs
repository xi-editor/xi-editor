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

use language_server::LanguageServerClient;
use serde_json;
use serde_json::Value;
use std;
use url::ParseError as URLParseError;
use jsonrpc_lite::Error as JsonRPCError;

use std::option::NoneError;

pub enum LSPHeader {
    ContentType,
    ContentLength(usize),
}

pub trait Callable: Send {
    fn call(self: Box<Self>, client: &mut LanguageServerClient, result: Result<Value, JsonRPCError>);
}

impl<F: Send + FnOnce(&mut LanguageServerClient, Result<Value, JsonRPCError>)> Callable for F {
    fn call(self: Box<F>, client: &mut LanguageServerClient, result: Result<Value, JsonRPCError>) {
        (*self)(client, result)
    }
}

pub type Callback = Box<Callable>;

// Error Types
#[derive(Debug)]
pub enum ParseError {
    Io(std::io::Error),
    ParseInt(std::num::ParseIntError),
    Utf8(std::string::FromUtf8Error),
    Json(serde_json::Error),
    Unknown(String),
}

impl From<std::io::Error> for ParseError {
    fn from(err: std::io::Error) -> ParseError {
        ParseError::Io(err)
    }
}

impl From<std::string::FromUtf8Error> for ParseError {
    fn from(err: std::string::FromUtf8Error) -> ParseError {
        ParseError::Utf8(err)
    }
}

impl From<serde_json::Error> for ParseError {
    fn from(err: serde_json::Error) -> ParseError {
        ParseError::Json(err)
    }
}

impl From<std::num::ParseIntError> for ParseError {
    fn from(err: std::num::ParseIntError) -> ParseError {
        ParseError::ParseInt(err)
    }
}

impl From<String> for ParseError {
    fn from(s: String) -> ParseError {
        ParseError::Unknown(s)
    }
}

pub enum Error {
    NoneError,
    URLParseError(URLParseError)
}

impl From<NoneError> for Error {
    fn from(_err: NoneError) -> Error {
        Error::NoneError
    }
}

impl From<URLParseError> for Error {
    fn from(err: URLParseError) -> Error {
        Error::URLParseError(err)
    }
}