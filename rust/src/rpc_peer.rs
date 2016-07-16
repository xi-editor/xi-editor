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

use std::collections::BTreeMap;
use std::io;
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};

use serde_json;
use serde_json::builder::ObjectBuilder;
use serde_json::Value;

pub struct RpcWriter<W: Write>(Arc<Mutex<W>>);

pub struct RpcPeer<R: BufRead, W: Write> {
    reader: R,
    buf: String,
    writer: RpcWriter<W>,
}

fn parse_rpc_request(json: &Value) -> Option<(Option<&Value>, &str, &Value)> {
    json.as_object().and_then(|req| {
        if let (Some(method), Some(params)) =
            (dict_get_string(req, "method"), req.get("params")) {
                let id = req.get("id");
                Some((id, method, params))
            }
        else { None }
    })
}

impl<R: BufRead, W:Write> RpcPeer<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        let rpc_writer = RpcWriter(Arc::new(Mutex::new(writer)));
        RpcPeer { reader: reader, buf: String::new(), writer: rpc_writer }
    }

    pub fn get_writer(&self) -> RpcWriter<W> {
        RpcWriter(self.writer.0.clone())
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

    pub fn mainloop<F: FnMut(&str, &Value) -> Option<Value>>(&mut self, mut f: F) {
        while let Some(json_result) = self.read_json() {
            match json_result {
                Ok(json) => {
                    print_err!("to core: {:?}", json);
                    match parse_rpc_request(&json) {
                        Some((id, method, params)) => {
                            if let Some(result) = f(method, params) {
                                self.writer.respond(&result, id);
                            } else if let Some(id) = id {
                                print_err!("RPC with id={:?} not responded", id);
                            }
                        }
                        None => print_err!("invalid RPC request")
                    }
                },
                Err(err) => print_err!("Error decoding json: {:?}", err)
            }
        }
    }
}

impl<W:Write> RpcWriter<W> {
    fn send(&self, v: &Value) -> Result<(), io::Error> {
        let mut s = serde_json::to_string(v).unwrap();
        s.push('\n');
        //print_err!("from core: {}", s);
        self.0.lock().unwrap().write_all(s.as_bytes())
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

fn dict_get_string<'a>(dict: &'a BTreeMap<String, Value>, key: &str) -> Option<&'a str> {
    dict.get(key).and_then(Value::as_string)
}
