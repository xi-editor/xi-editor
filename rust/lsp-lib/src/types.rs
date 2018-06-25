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

use std::collections::HashMap;
use language_server_client::LanguageServerClient;
use serde_json;
use serde_json::Value;
use std;
use url::ParseError as UrlParseError;
use std::io::Error as IOError;
use jsonrpc_lite::Error as JsonRpcError;


pub enum LspHeader {
    ContentType,
    ContentLength(usize),
}

pub trait Callable: Send {
    fn call(self: Box<Self>, client: &mut LanguageServerClient, result: Result<Value, JsonRpcError>);
}

impl<F: Send + FnOnce(&mut LanguageServerClient, Result<Value, JsonRpcError>)> Callable for F {
    fn call(self: Box<F>, client: &mut LanguageServerClient, result: Result<Value, JsonRpcError>) {
        (*self)(client, result)
    }
}

pub type Callback = Box<Callable>;

#[derive(Serialize, Deserialize)]
/// Language Specific Configuration
pub struct LanguageConfig {
    pub language_name: String,
    pub start_command: String,
    pub start_arguments: Vec<String>,
    pub extensions: Vec<String>,
    pub supports_single_file: bool,
    pub workspace_identifier: Option<String>
}

/// Represents the config for the Language Plugin
#[derive(Serialize, Deserialize)]
pub struct Config {
    pub language_config: HashMap<String, LanguageConfig>
}


// Error Types

/// Type to represent errors occured while parsing LSP RPCs
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

// TODO: Improve Error handling in module and add more types as necessary

/// Types to represent errors in the module.
pub enum Error {
    PathError,
    FileUrlParseError,
    IOError(IOError),
    UrlParseError(UrlParseError)
}

impl From<UrlParseError> for Error {
    fn from(err: UrlParseError) -> Error {
        Error::UrlParseError(err)
    }
}

impl From<IOError> for Error {
    fn from(err: IOError) -> Error {
        Error::IOError(err)
    }
}