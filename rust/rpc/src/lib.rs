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

extern crate serde;
extern crate serde_json;
extern crate crossbeam;

#[macro_use]
mod macros;

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex, Condvar};
use std::sync::mpsc;
use std::sync::atomic::{AtomicUsize, Ordering};
use crossbeam::scope;

use serde_json::builder::ObjectBuilder;
use serde_json::Value;

pub struct RpcPeer<W: Write>(Arc<RpcState<W>>);

struct RpcState<W: Write> {
    rx_queue: Mutex<VecDeque<Value>>,
    rx_cvar: Condvar,
    writer: Mutex<W>,
    id: AtomicUsize,
    pending: Mutex<BTreeMap<usize, mpsc::Sender<Value>>>,
}

pub struct RpcLoop<R: BufRead, W: Write> {
    reader: R,
    buf: String,
    peer: RpcPeer<W>,
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

impl<R: BufRead, W:Write + Send> RpcLoop<R, W> {
    pub fn new(reader: R, peer: W) -> Self {
        let rpc_peer = RpcPeer(Arc::new(RpcState {
            rx_queue: Mutex::new(VecDeque::new()),
            rx_cvar: Condvar::new(),
            writer: Mutex::new(peer),
            id: AtomicUsize::new(0),
            pending: Mutex::new(BTreeMap::new()),
        }));
        RpcLoop {
            reader: reader,
            buf: String::new(),
            peer: rpc_peer,
        }
    }

    pub fn get_peer(&self) -> RpcPeer<W> {
        RpcPeer(self.peer.0.clone())
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

    pub fn mainloop<F: FnMut(&str, &Value) -> Option<Value> + Send>(&mut self, mut f: F) {
        crossbeam::scope(|scope| {
            let peer = self.get_peer();
            scope.spawn(move|| {
                loop {
                    let json = peer.get_rx();
                    if json == Value::Null {
                        break;
                    }
                    print_err!("to core: {:?}", json);
                    match parse_rpc_request(&json) {
                        Some((id, method, params)) => {
                            if let Some(result) = f(method, params) {
                                peer.respond(&result, id);
                            } else if let Some(id) = id {
                                print_err!("RPC with id={:?} not responded", id);
                            }
                        }
                        None => print_err!("invalid RPC request")
                    }                
                }
            });
            while let Some(json_result) = self.read_json() {
                match json_result {
                    Ok(json) => {
                        let is_method = json.as_object().map_or(false, |dict| dict.contains_key("method"));
                        if is_method {
                            self.peer.put_rx(json)
                        } else {
                            self.peer.handle_response(json)
                        }
                    }
                    Err(err) => print_err!("Error decoding json: {:?}", err)
                }
            }
            self.peer.put_rx(Value::Null);
        });
    }
}

impl<W:Write> RpcPeer<W> {
    fn send(&self, v: &Value) -> Result<(), io::Error> {
        let mut s = serde_json::to_string(v).unwrap();
        s.push('\n');
        //print_err!("from core: {}", s);
        self.0.writer.lock().unwrap().write_all(s.as_bytes())
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

    pub fn send_rpc_sync(&self, method: &str, params: &Value) -> Value {
        let id = self.0.id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        {
            let mut pending = self.0.pending.lock().unwrap();
            pending.insert(id, tx);
        }
        if let Err(e) = self.send(&ObjectBuilder::new()
            .insert("id", Value::U64(id as u64))
            .insert("method", method)
            .insert("params", params)
            .unwrap()) {
            print_err!("send error on send_rpc_async method {}: {}", method, e);
            panic!("TODO: better error handling");
        }
        rx.recv().unwrap()
    }

    fn handle_response(&self, mut response: Value) {
        let mut dict = response.as_object_mut().unwrap();
        let result = dict.remove("result").unwrap();
        let id = dict.get("id").and_then(Value::as_u64).unwrap() as usize;
        let mut pending = self.0.pending.lock().unwrap();
        match pending.remove(&id) {
            Some(tx) => {
                let _  = tx.send(result);
            }
            None => print_err!("id {} not found in pending", id)
        }
    }

    fn get_rx(&self) -> Value {
        let mut queue = self.0.rx_queue.lock().unwrap();
        while queue.is_empty() {
            queue = self.0.rx_cvar.wait(queue).unwrap();
        }
        queue.pop_front().unwrap()
    }

    fn put_rx(&self, json: Value) {
        let mut queue = self.0.rx_queue.lock().unwrap();
        queue.push_back(json);
        self.0.rx_cvar.notify_one();
    }

    pub fn request_is_pending(&self) -> bool {
        let queue = self.0.rx_queue.lock().unwrap();
        !queue.is_empty()
    }
}

fn dict_get_string<'a>(dict: &'a BTreeMap<String, Value>, key: &str) -> Option<&'a str> {
    dict.get(key).and_then(Value::as_string)
}
