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

//! Generic RPC handling (used for both front end and plugin communication).

use std::cell::RefCell;
use std::io;
use std::io::{BufRead, Write};

use serde_json;
use serde_json::builder::ObjectBuilder;
use serde_json::Value;

pub struct RpcPeer<R: BufRead, W: Write> {
    reader: R,
    buf: String,
    writer: RefCell<W>,
}

impl<R: BufRead, W:Write> RpcPeer<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        RpcPeer { reader: reader, buf: String::new(), writer: RefCell::new(writer) }
    }

    pub fn read_json(&mut self) -> Option<serde_json::error::Result<Value>> {
        self.buf.clear();
        if self.reader.read_line(&mut self.buf).is_ok() {
            if self.buf.is_empty() {
                return None;
            }
            return Some(serde_json::from_str::<Value>(&self.buf));
        }
        None
    }

    fn send(&self, v: &Value) -> Result<(), io::Error> {
        let mut s = serde_json::to_string(v).unwrap();
        s.push('\n');
        //print_err!("from core: {}", s);
        self.writer.borrow_mut().write_all(s.as_bytes())
        // Technically, maybe we should flush here, but doesn't seem to be reqiured.
    }

    pub fn respond(&self, result: &Value, id: Option<&Value>) {
        if let Some(id) = id {
            if let Err(e) = self.send(&ObjectBuilder::new()
                                 .insert("id", id)
                                 .insert("result", result)
                                 .unwrap()) {
                print_err!("error {} sending response to RPC {:?}", e, id);
            }
        } else {
            print_err!("tried to respond with no id");
        }
    }

    pub fn send_rpc_async(&self, method: &str, params: &Value) {
        if let Err(e) = self.send(&ObjectBuilder::new()
            .insert("method", method)
            .insert("params", params)
            .unwrap()) {
            print_err!("send error on send_rpc_async method {}: {}", method, e);
        }
    }
}
