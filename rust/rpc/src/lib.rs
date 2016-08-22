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
//!
//! The RPC protocol is based on [JSON-RPC](http://www.jsonrpc.org/specification),
//! but with some modifications. Unlike JSON-RPC 2.0, requests and notifications
//! are allowed in both directions, rather than imposing client and server roles.
//! Further, the batch form is not supported.
//!
//! Because these changes make the protocol not fully compliant with the spec,
//! the `"jsonrpc"` member is omitted from request and response objects.

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

#[derive(Debug)]
pub enum Error {
    /// An IO error occurred on the underlying communication channel.
    IoError(io::Error),
    /// The peer closed its connection.
    PeerDisconnect,
    /// The remote method returned an error.
    RemoteError(Value),
    /// The peer sent a response containing the id, but was malformed according
    /// to the json-rpc spec.
    MalformedResponse,
}

/// An interface to access the other side of the RPC channel. The main purpose
/// is to send RPC requests and notifications to the peer.
///
/// The concrete type may change; if the `RpcLoop` were to start a separate
/// writer thread, as opposed to writes being synchronous, then the peer would
/// not need to take the writer type as a parameter.
pub struct RpcPeer<W: Write>(Arc<RpcState<W>>);

pub struct RpcCtx<'a, W: 'a + Write> {
    peer: &'a RpcPeer<W>,
    idle: &'a mut VecDeque<usize>,
}

pub trait Handler<W: Write> {
    fn handle_notification(&mut self, ctx: RpcCtx<W>, method: &str, params: &Value);
    fn handle_request(&mut self, ctx: RpcCtx<W>, method: &str, params: &Value) ->
        Result<Value, Value>;
    #[allow(unused_variables)]
    fn idle(&mut self, ctx: RpcCtx<W>, token: usize) {}
}

trait Callback: Send {
    fn call(self: Box<Self>, result: Result<Value, Error>);
}

impl<F:Send + FnOnce(Result<Value, Error>)> Callback for F {
    fn call(self: Box<F>, result: Result<Value, Error>) {
        (*self)(result)
    }
}

trait IdleProc: Send {
    fn call(self: Box<Self>, token: usize);
}

impl<F:Send + FnOnce(usize)> IdleProc for F {
    fn call(self: Box<F>, token: usize) {
        (*self)(token)
    }
}

enum ResponseHandler {
    Chan(mpsc::Sender<Result<Value, Error>>),
    Callback(Box<Callback>),
}

impl ResponseHandler {
    fn invoke(self, result: Result<Value, Error>) {
        match self {
            ResponseHandler::Chan(tx) => {
                let _ = tx.send(result);
            },
            ResponseHandler::Callback(f) => f.call(result)
        }
    }
}

struct RpcState<W: Write> {
    rx_queue: Mutex<VecDeque<Value>>,
    rx_cvar: Condvar,
    writer: Mutex<W>,
    id: AtomicUsize,
    pending: Mutex<BTreeMap<usize, ResponseHandler>>,
}

/// A structure holding the state of a main loop for handing RPC's.
pub struct RpcLoop<W: Write> {
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

impl<W:Write + Send> RpcLoop<W> {
    /// Creates a new `RpcLoop` with the given output stream (which is used for
    /// sending requests and notifications, as well as responses).
    pub fn new(writer: W) -> Self {
        let rpc_peer = RpcPeer(Arc::new(RpcState {
            rx_queue: Mutex::new(VecDeque::new()),
            rx_cvar: Condvar::new(),
            writer: Mutex::new(writer),
            id: AtomicUsize::new(0),
            pending: Mutex::new(BTreeMap::new()),
        }));
        RpcLoop {
            buf: String::new(),
            peer: rpc_peer,
        }
    }

    /// Gets a reference to the peer.
    pub fn get_peer(&self) -> RpcPeer<W> {
        self.peer.clone()
    }

    // Reads raw json from the input stream.
    fn read_json<R: BufRead>(&mut self, reader: &mut R)
            -> Option<serde_json::error::Result<Value>> {
        self.buf.clear();
        if reader.read_line(&mut self.buf).is_ok() {
            if self.buf.is_empty() {
                return None;
            }
            return Some(serde_json::from_str::<Value>(&self.buf));
        }
        None
    }

    /// Starts a main loop. The reader is supplied via a closure, as basically
    /// a workaround so that the reader doesn't have to be `Send`. Internally, the
    /// main loop starts a separate thread for I/O, and at startup that thread calls
    /// the given closure.
    ///
    /// Calls to the handler (the second closure) happen on the caller's thread, so
    /// that closure need not be `Send`.
    ///
    /// Calls to the handler are guaranteed to preserve the order as they appear on
    /// on the channel. At the moment, there is no way for there to be more than one
    /// incoming request to be outstanding.
    ///
    /// This method returns when the input channel is closed.
    pub fn mainloop<R: BufRead, RF: Send + FnOnce() -> R>(&mut self,
            rf: RF,
            handler: &mut Handler<W>) {
        crossbeam::scope(|scope| {
            let peer = self.get_peer();
            scope.spawn(move|| {
                let mut reader = rf();
                while let Some(json_result) = self.read_json(&mut reader) {
                    match json_result {
                        Ok(json) => {
                            let is_method = json.as_object().map_or(false, |dict|
                                dict.contains_key("method"));
                            if is_method {
                                self.peer.put_rx(json);
                            } else {
                                self.peer.handle_response(json);
                            }
                        }
                        Err(err) => print_err!("Error decoding json: {:?}", err)
                    }
                }
                self.peer.put_rx(Value::Null);
                // TODO: send disconnect error to all pending
            });
            let mut idle = VecDeque::<usize>::new();
            loop {
                let json = if !idle.is_empty() {
                    if let Some(json) = peer.try_get_rx() {
                        json
                    } else {
                        let token = idle.pop_front().unwrap();
                        let ctx = RpcCtx {
                            peer: &peer,
                            idle: &mut idle,
                        };
                        handler.idle(ctx, token);
                        continue;
                    }
                } else {
                    peer.get_rx()
                };
                if json == Value::Null {
                    break;
                }
                //print_err!("to core: {:?}", json);
                match parse_rpc_request(&json) {
                    Some((id, method, params)) => {
                        let ctx = RpcCtx {
                            peer: &peer,
                            idle: &mut idle,
                        };
                        if let Some(id) = id {
                            let result = handler.handle_request(ctx, method, params);
                            peer.respond(result, id);
                        } else {
                            handler.handle_notification(ctx, method, params);
                        }
                    }
                    None => print_err!("invalid RPC request")
                }
            }
        });
    }
}

impl<'a, W: Write> RpcCtx<'a, W> {
    pub fn get_peer(&self) -> &RpcPeer<W> {
        self.peer
    }

    /// Schedule the idle handler to be run when there are no requests pending.
    pub fn schedule_idle(&mut self, token: usize) {
        self.idle.push_back(token);
    }
}

impl<W:Write> RpcPeer<W> {
    fn send(&self, v: &Value) -> Result<(), io::Error> {
        let mut s = serde_json::to_string(v).unwrap();
        s.push('\n');
        //print_err!("from core: {}", s);
        self.0.writer.lock().unwrap().write_all(s.as_bytes())
        // Technically, maybe we should flush here, but doesn't seem to be required.
    }

    fn respond(&self, result: Result<Value, Value>, id: &Value) {
        let mut builder = ObjectBuilder::new()
            .insert("id", id);
        match result {
            Ok(result) => builder = builder.insert("result", result),
            Err(error) => builder = builder.insert("error", error),
        }
        if let Err(e) = self.send(&builder.build()) {
            print_err!("error {} sending response to RPC {:?}", e, id);
        }
    }

    /// Sends a notification (asynchronous rpc) to the peer.
    pub fn send_rpc_notification(&self, method: &str, params: &Value) {
        if let Err(e) = self.send(&ObjectBuilder::new()
            .insert("method", method)
            .insert("params", params)
            .build()) {
            print_err!("send error on send_rpc_notification method {}: {}", method, e);
        }
    }

    fn send_rpc_request_common(&self, method: &str, params: &Value, rh: ResponseHandler) {
        let id = self.0.id.fetch_add(1, Ordering::Relaxed);
        {
            let mut pending = self.0.pending.lock().unwrap();
            pending.insert(id, rh);
        }
        if let Err(e) = self.send(&ObjectBuilder::new()
                .insert("id", id)
                .insert("method", method)
                .insert("params", params)
                .build()) {
            let mut pending = self.0.pending.lock().unwrap();
            if let Some(rh) = pending.remove(&id) {
                rh.invoke(Err(Error::IoError(e)));
            }
        }
    }

    /// Sends a request asynchronously, and the supplied callback will be called when
    /// the response arrives.
    pub fn send_rpc_request_async<F>(&self, method: &str, params: &Value, f: F)
        where F: FnOnce(Result<Value, Error>) + Send + 'static {
        self.send_rpc_request_common(method, params, ResponseHandler::Callback(Box::new(f)));
    }

    /// Sends a request (synchronous rpc) to the peer, and waits for the result.
    pub fn send_rpc_request(&self, method: &str, params: &Value) -> Result<Value, Error> {
        let (tx, rx) = mpsc::channel();
        self.send_rpc_request_common(method, params, ResponseHandler::Chan(tx));
        rx.recv().unwrap_or(Err(Error::PeerDisconnect))
    }

    fn handle_response(&self, mut response: Value) {
        let mut dict = response.as_object_mut().unwrap();
        let id = dict.get("id").and_then(Value::as_u64);
        if id.is_none() {
            print_err!("id missing from response, or is not u64");
            return;
        }
        let id = id.unwrap() as usize;
        let result = dict.remove("result");
        let error = dict.remove("error");
        let result = match (result, error) {
            (Some(result), None) => Ok(result),
            (None, Some(err)) => Err(Error::RemoteError(err)),
            _ => Err(Error::MalformedResponse)
        };
        let handler = {
            let mut pending = self.0.pending.lock().unwrap();
            pending.remove(&id)
        };
        match handler {
            Some(responsehandler) => responsehandler.invoke(result),
            None => print_err!("id {} not found in pending", id)
        }
    }

    // Get a message from the recieve queue if available.
    fn try_get_rx(&self) -> Option<Value> {
        let mut queue = self.0.rx_queue.lock().unwrap();
        queue.pop_front()
    }

    // Get a message from the receive queue, blocking until available.
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

    /// Determines whether an incoming request (or notification) is pending. This
    /// is intended to reduce latency for bulk operations done in the background;
    /// the handler can do this work, periodically check
    pub fn request_is_pending(&self) -> bool {
        let queue = self.0.rx_queue.lock().unwrap();
        !queue.is_empty()
    }
}

impl<W:Write> Clone for RpcPeer<W> {
    fn clone(&self) -> Self {
        RpcPeer(self.0.clone())
    }
}

// =============================================================================
//  Helper functions for value access
// =============================================================================

pub fn dict_get_u64(dict: &BTreeMap<String, Value>, key: &str) -> Option<u64> {
    dict.get(key).and_then(Value::as_u64)
}

pub fn dict_get_string<'a>(dict: &'a BTreeMap<String, Value>, key: &str) -> Option<&'a str> {
    dict.get(key).and_then(Value::as_str)
}

pub fn arr_get_u64(arr: &[Value], idx: usize) -> Option<u64> {
    arr.get(idx).and_then(Value::as_u64)
}

pub fn arr_get_i64(arr: &[Value], idx: usize) -> Option<i64> {
    arr.get(idx).and_then(Value::as_i64)
}
