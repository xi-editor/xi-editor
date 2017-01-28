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
use std::fmt;

use serde_json::Value;
use serde_json::builder::ObjectBuilder;

use xi_rpc;
use xi_rpc::{RpcLoop, RpcCtx, dict_get_u64, dict_get_string};

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

// TODO: make more similar to xi_rpc::Handler
pub trait Handler {
    fn call(&mut self, &PluginRequest, PluginCtx) -> Option<Value>;
    #[allow(unused_variables)]
    fn idle(&mut self, ctx: PluginCtx, token: usize) {}
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
            .build());
    }

    pub fn build(self) -> Spans {
        Value::Array(self.0)
    }
}

pub struct PluginCtx<'a>(RpcCtx<'a, io::Stdout>);

impl<'a> PluginCtx<'a> {
    /*
    // Not used.
    pub fn n_lines(&self) -> Result<usize, Error> {
        let result = self.send_rpc_request("n_lines", &Value::Array(vec![]));
        match result {
            Ok(value) => value.as_u64().map(|value| value as usize).ok_or(Error::WrongReturnType),
            Err(err) => Err(Error::RpcError(err)),
        }
    }
    */

    /*
    // Obsolete, superseded by get_data.
    pub fn get_line(&self, line_num: usize) -> Result<String, Error> {
        let params = ObjectBuilder::new().insert("line", Value::U64(line_num as u64)).build();
        let result = self.send_rpc_request("get_line", &params);
        match result {
            Ok(Value::String(s)) => Ok(s),
            Ok(_) => Err(Error::WrongReturnType),
            Err(err) => Err(Error::RpcError(err)),
        }
    }
    */

    pub fn get_data(&self, offset: usize, max_size: usize, rev: usize) -> Result<String, Error> {
        let params = ObjectBuilder::new()
            .insert("offset", offset)
            .insert("max_size", max_size)
            .insert("rev", rev)
            .build();
        let result = self.send_rpc_request("get_data", &params);
        match result {
            Ok(Value::String(s)) => Ok(s),
            Ok(_) => Err(Error::WrongReturnType),
            Err(err) => Err(Error::RpcError(err)),
        }
    }

    pub fn set_fg_spans(&self, start: usize, len: usize, spans: Spans, rev: usize) {
        let params = ObjectBuilder::new()
            .insert("start", start)
            .insert("len", len)
            .insert("spans", spans)
            .insert("rev", rev)
            .build();
        self.send_rpc_notification("set_fg_spans", &params);
    }

    fn send_rpc_notification(&self, method: &str, params: &Value) {
        self.0.get_peer().send_rpc_notification(method, params)
    }

    fn send_rpc_request(&self, method: &str, params: &Value) -> Result<Value, xi_rpc::Error> {
        self.0.get_peer().send_rpc_request(method, params)
    }

    /// Determines whether an incoming request (or notification) is pending. This
    /// is intended to reduce latency for bulk operations done in the background.
    pub fn request_is_pending(&self) -> bool {
        self.0.get_peer().request_is_pending()
    }

    /// Schedule the idle handler to be run when there are no requests pending.
    pub fn schedule_idle(&mut self, token: usize) {
        self.0.schedule_idle(token);
    }
}

#[derive(Debug, Clone, Copy)]
pub enum EditType {
    Insert,
    Delete,
    Undo,
    Redo,
    Other,
}

impl EditType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "insert" => EditType::Insert,
            "delete" => EditType::Delete,
            "undo" => EditType::Undo,
            "redo" => EditType::Redo,
            _ => EditType::Other,
        }
    }
}

pub enum PluginRequest<'a> {
    Ping,
    InitBuf {
        buf_size: usize,
        rev: usize,
    },
    Update {
        start: usize,
        end: usize,
        new_len: usize,
        rev: usize,
        edit_type: EditType,
        text: Option<&'a str>,
    },
}

enum InternalError {
    InvalidParams,
    UnknownMethod(String),
}

impl fmt::Display for InternalError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            InternalError::UnknownMethod(ref method) => write!(f, "Unknown method {}", method),
            InternalError::InvalidParams => write!(f, "Invalid params"),
        }
    }
}

fn parse_plugin_request<'a>(method: &str, params: &'a Value) ->
        Result<PluginRequest<'a>, InternalError> {
    match method {
        "ping" => Ok(PluginRequest::Ping),
        "init_buf" => {
            params.as_object().and_then(|dict|
                if let (Some(buf_size), Some(rev)) = 
                    (dict_get_u64(dict, "buf_size"), dict_get_u64(dict, "rev")) {
                        Some(PluginRequest::InitBuf {
                            buf_size: buf_size as usize,
                            rev: rev as usize,
                        })
                } else { None }
            ).ok_or_else(|| InternalError::InvalidParams)
        }
        "update" => {
            params.as_object().and_then(|dict|
                if let (Some(start), Some(end), Some(new_len), Some(rev), Some(edit_type)) =
                    (dict_get_u64(dict, "start"), dict_get_u64(dict, "end"),
                        dict_get_u64(dict, "new_len"), dict_get_u64(dict, "rev"),
                        dict_get_string(dict, "edit_type")) {
                        Some(PluginRequest::Update {
                            start: start as usize,
                            end: end as usize,
                            new_len: new_len as usize,
                            rev: rev as usize,
                            edit_type: EditType::from_str(edit_type),
                            text: dict_get_string(dict, "text"),
                        })
                } else { None }
            ).ok_or_else(|| InternalError::InvalidParams)
        }
        _ => Err(InternalError::UnknownMethod(method.to_string()))
    }
}

struct MyHandler<'a, H: 'a>(&'a mut H);

impl<'a, H: Handler> xi_rpc::Handler<io::Stdout> for MyHandler<'a, H> {
    fn handle_notification(&mut self, ctx: RpcCtx<io::Stdout>, method: &str, params: &Value) {
        match parse_plugin_request(method, params) {
            Ok(req) => {
                let _ = self.0.call(&req, PluginCtx(ctx));
                // TODO: should check None
            }
            Err(err) => print_err!("error: {}", err)
        }
    }

    fn handle_request(&mut self, ctx: RpcCtx<io::Stdout>, method: &str, params: &Value) ->
        Result<Value, Value> {
        match parse_plugin_request(method, params) {
            Ok(req) => {
                let result = self.0.call(&req, PluginCtx(ctx));
                result.ok_or_else(|| Value::String("return value missing".to_string()))
            }
            Err(err) => {
                print_err!("Error {} decoding RPC request {}", err, method);
                Err(Value::String("error decoding request".to_string()))
            }
        }
    }

    fn idle(&mut self, ctx: RpcCtx<io::Stdout>, token: usize) {
        self.0.idle(PluginCtx(ctx), token);
    }
}

pub fn mainloop<H: Handler>(handler: &mut H) {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut rpc_looper = RpcLoop::new(stdout);
    let mut my_handler = MyHandler(handler);

    rpc_looper.mainloop(|| stdin.lock(), &mut my_handler);
}
