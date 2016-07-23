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

pub struct SpansBuilder(Vec<Value>);
pub type Spans = Value;

impl SpansBuilder {
    pub fn new() -> Self {
        SpansBuilder(Vec::new())
    }

    pub fn add_fg_span(&mut self, start: usize, end: usize, fg: u32) {
        self.0.push(ObjectBuilder::new()
            .insert("start", start as u64)
            .insert("end", end as u64)
            .insert("fg", fg as u64)
            .unwrap());
    }

    pub fn build(self) -> Spans {
        Value::Array(self.0)
    }
}

pub struct PluginPeer(RpcPeer<io::Stdout>);

impl PluginPeer {
    pub fn n_lines(&self) -> usize {
        let result = self.send_rpc_request("n_lines", &Value::Array(vec![]));
        result.as_u64().unwrap() as usize
    }

    pub fn get_line(&self, line_num: usize) -> String {
        let params = ObjectBuilder::new().insert("line", Value::U64(line_num as u64)).unwrap();
        let result = self.send_rpc_request("get_line", &params);
        match result {
            Value::String(s) => s,
            _ => panic!("wrong return type of get_line")
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
        self.0.send_rpc_async(method, params)
    }

    fn send_rpc_request(&self, method: &str, params: &Value) -> Value {
        self.0.send_rpc_sync(method, params)
    }
}

pub enum PluginRequest {
    Ping,
    PingFromEditor,
}

enum Error {
    UnknownMethod(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Error::UnknownMethod(ref method) => write!(f, "Unknown method {}", method),
        }
    }
}

fn parse_plugin_request(method: &str, _params: &Value) -> Result<PluginRequest, Error> {
    match method {
        "ping" => Ok(PluginRequest::Ping),
        "ping_from_editor" => Ok(PluginRequest::PingFromEditor),
        _ => Err(Error::UnknownMethod(method.to_string()))
    }
}

pub fn mainloop<F: FnMut(&PluginRequest, &PluginPeer) -> Option<Value> + Send>(mut f: F) {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut rpc_looper = RpcLoop::new(stdin.lock(), stdout);
    let peer = PluginPeer(rpc_looper.get_peer());

    rpc_looper.mainloop(|method, params|
        match parse_plugin_request(method, params) {
            Ok(req) => f(&req, &peer),
            Err(err) => {
                print_err!("error: {}", err);
                None
            }
        }
    );
}
