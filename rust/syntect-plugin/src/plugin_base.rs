// Copyright 2016 Google Inc. All rights reserved.
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

//! A base for xi plugins. Will be split out into its own crate once it's a bit more stable.

use std::io;
use std::io::Write;
use std::fmt;

use serde_json::Value;
use serde_json::builder::ObjectBuilder;

use xi_rpc;
use xi_rpc::{RpcLoop, RpcPeer};

// TODO: avoid duplicating this in every crate
macro_rules! print_err {
    ($($arg:tt)*) => (
        {
            use std::io::prelude::*;
            if let Err(e) = write!(&mut ::std::io::stderr(), "{}\n", format_args!($($arg)*)) {
                panic!("Failed to write to stderr.\
                    \nOriginal error output: {}\
                    \nSecondary error writing to stderr: {}", format!($($arg)*), e);
            }
        }
    )
}

#[derive(Debug)]
pub enum Error {
    RpcError(xi_rpc::Error),
    WrongReturnType,
} 

pub struct SpansBuilder(Vec<Value>);
pub type Spans = Value;

impl SpansBuilder {
    pub fn new() -> Self {
        SpansBuilder(Vec::new())
    }

    pub fn add_style_span(&mut self, start: usize, end: usize, fg: u32, font_style: u8) {
        self.0.push(ObjectBuilder::new()
            .insert("start", start as u64)
            .insert("end", end as u64)
            .insert("fg", fg as u64)
            .insert("font", font_style as u64)
            .unwrap());
    }

    pub fn build(self) -> Spans {
        Value::Array(self.0)
    }
}

pub struct PluginPeer(RpcPeer<io::Stdout>);

impl PluginPeer {
    pub fn n_lines(&self) -> Result<usize, Error> {
        let result = self.send_rpc_request("n_lines", &Value::Array(vec![]));
        match result {
            Ok(value) => value.as_u64().map(|value| value as usize).ok_or(Error::WrongReturnType),
            Err(err) => Err(Error::RpcError(err)),
        }
    }

    pub fn get_line(&self, line_num: usize) -> Result<String, Error> {
        let params = ObjectBuilder::new().insert("line", Value::U64(line_num as u64)).unwrap();
        let result = self.send_rpc_request("get_line", &params);
        match result {
            Ok(Value::String(s)) => Ok(s),
            Ok(_) => Err(Error::WrongReturnType),
            Err(err) => Err(Error::RpcError(err)),
        }
    }

    pub fn set_line_fg_spans(&self, line_num: usize, spans: Spans) {
        let params = ObjectBuilder::new()
            .insert("line", Value::U64(line_num as u64))
            .insert("spans", spans)
            .unwrap();
        self.send_rpc_notification("set_line_fg_spans", &params);
    }

    fn send_rpc_notification(&self, method: &str, params: &Value) {
        self.0.send_rpc_notification(method, params)
    }

    fn send_rpc_request(&self, method: &str, params: &Value) -> Result<Value, xi_rpc::Error> {
        self.0.send_rpc_request(method, params)
    }
}

pub enum PluginRequest {
    Ping,
    PingFromEditor,
}

enum InternalError {
    UnknownMethod(String),
}

impl fmt::Display for InternalError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            InternalError::UnknownMethod(ref method) => write!(f, "Unknown method {}", method),
        }
    }
}

fn parse_plugin_request(method: &str, _params: &Value) -> Result<PluginRequest, InternalError> {
    match method {
        "ping" => Ok(PluginRequest::Ping),
        "ping_from_editor" => Ok(PluginRequest::PingFromEditor),
        _ => Err(InternalError::UnknownMethod(method.to_string()))
    }
}

pub fn mainloop<F: FnMut(&PluginRequest, &PluginPeer) -> Option<Value>>(mut f: F) {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut rpc_looper = RpcLoop::new(stdout);
    let peer = PluginPeer(rpc_looper.get_peer());

    rpc_looper.mainloop(|| stdin.lock(),
        |method, params|
        match parse_plugin_request(method, params) {
            Ok(req) => f(&req, &peer),
            Err(err) => {
                print_err!("error: {}", err);
                None
            }
        }
    );
}
