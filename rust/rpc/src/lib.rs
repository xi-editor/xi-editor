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

#[macro_use]
extern crate serde_json;
#[macro_use]
extern crate serde_derive;
extern crate serde;
extern crate crossbeam;

#[macro_use]
mod macros;
mod parse;
mod error;

pub mod test_utils;

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, BufRead, Write};
use std::sync::{Arc, Mutex, Condvar};
use std::sync::mpsc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;
use serde::de::DeserializeOwned;

use parse::{Call, Response, RpcObject, MessageReader};
pub use error::{Error, ReadError, RemoteError};


/// An interface to access the other side of the RPC channel. The main purpose
/// is to send RPC requests and notifications to the peer.
///
/// A single shared `RawPeer` exists for each `RpcLoop`; a reference can
/// be taken with `RpcLoop::get_peer()`.
///
/// In general, `RawPeer` shouldn't be used directly, but behind a pointer as
/// the `Peer` trait object.
pub struct RawPeer<W: Write + 'static>(Arc<RpcState<W>>);

/// The `Peer` trait represents the interface for the other side of the RPC
/// channel. It is intended to be used behind a pointer, a trait object.
pub trait Peer: Send + 'static {
    /// Used to implement `clone` in an object-safe way.
    /// For an explanation on this approach, see this thread:
    /// https://users.rust-lang.org/t/solved-is-it-possible-to-clone-a-boxed-trait-object/1714/6
    fn box_clone(&self) -> Box<Peer>;
    /// Sends a notification (asynchronous RPC) to the peer.
    fn send_rpc_notification(&self, method: &str, params: &Value);
    /// Sends a request asynchronously, and the supplied callback will
    /// be called when the response arrives.
    ///
    /// `Callback` is an alias for FnOnce(Result<Value, Error>); it must
    /// be boxed because trait objects cannot use generic paramaters.
    fn send_rpc_request_async(&self, method: &str, params: &Value,
                              f: Box<Callback>);
    /// Sends a request (synchronous RPC) to the peer, and waits for the result.
    fn send_rpc_request(&self, method: &str, params: &Value)
                        -> Result<Value, Error>;
    /// Determines whether an incoming request (or notification) is
    /// pending. This is intended to reduce latency for bulk operations
    /// done in the background.
    fn request_is_pending(&self) -> bool;
    fn schedule_idle(&self, token: usize);
}

/// The `Peer` trait object.
pub type RpcPeer = Box<Peer>;

pub struct RpcCtx {
    peer: RpcPeer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// An RPC command.
///
/// This type is used as a placeholder in various places, and can be
/// used by clients as a catchall type for implementing `MethodHandler`.
pub struct RpcCall {
    pub method: String,
    pub params: Value,
}

/// A trait for types which can handle RPCs.
///
/// Types which implement `MethodHandler` are also responsible for implementing
/// `Parser`; `Parser` is provided when Self::Notification and Self::Request
/// can be used with serde::DeserializeOwned.
pub trait Handler {
    type Notification: DeserializeOwned;
    type Request: DeserializeOwned;
    fn handle_notification(&mut self, ctx: &RpcCtx, rpc: Self::Notification);
    fn handle_request(&mut self, ctx: &RpcCtx, rpc: Self::Request)
                      -> Result<Value, RemoteError>;
    #[allow(unused_variables)]
    fn idle(&mut self, ctx: &RpcCtx, token: usize) {}
}

pub trait Callback: Send {
    fn call(self: Box<Self>, result: Result<Value, Error>);
}

impl<F: Send + FnOnce(Result<Value, Error>)> Callback for F {
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
    rx_queue: Mutex<VecDeque<Result<RpcObject, ReadError>>>,
    rx_cvar: Condvar,
    writer: Mutex<W>,
    id: AtomicUsize,
    pending: Mutex<BTreeMap<usize, ResponseHandler>>,
    idle_queue: Mutex<VecDeque<usize>>,
}

/// A structure holding the state of a main loop for handling RPC's.
pub struct RpcLoop<W: Write + 'static> {
    reader: MessageReader,
    peer: RawPeer<W>,
}

impl<W: Write + Send> RpcLoop<W> {
    /// Creates a new `RpcLoop` with the given output stream (which is used for
    /// sending requests and notifications, as well as responses).
    pub fn new(writer: W) -> Self {
        let rpc_peer = RawPeer(Arc::new(RpcState {
            rx_queue: Mutex::new(VecDeque::new()),
            rx_cvar: Condvar::new(),
            writer: Mutex::new(writer),
            id: AtomicUsize::new(0),
            pending: Mutex::new(BTreeMap::new()),
            idle_queue: Mutex::new(VecDeque::new()),
        }));
        RpcLoop {
            reader: MessageReader::default(),
            peer: rpc_peer,
        }
    }

    /// Gets a reference to the peer.
    pub fn get_raw_peer(&self) -> RawPeer<W> {
        self.peer.clone()
    }

    /// Starts the event loop, reading lines from the reader until EOF,
    /// or an error occurs.
    ///
    /// Returns `Ok()` in  the EOF case, otherwise returns the
    /// underlying `ReadError`.
    ///
    /// # Note:
    /// The reader is supplied via a closure, as basically a workaround
    /// so that the reader doesn't have to be `Send`. Internally, the
    /// main loop starts a separate thread for I/O, and at startup that
    /// thread calls the given closure.
    ///
    /// Calls to the handler happen on the caller's thread.
    ///
    /// Calls to the handler are guaranteed to preserve the order as
    /// they appear on on the channel. At the moment, there is no way
    /// for there to be more than one incoming request to be outstanding.
    pub fn mainloop<'a, R, RF, H>(&mut self, rf: RF, handler: &mut H)
                                  -> Result<(), ReadError>
    where R: BufRead,
          RF: Send + FnOnce() -> R,
          H: Handler,
    {

        let exit = crossbeam::scope(|scope| {
            let peer = self.get_raw_peer();
            let ctx = RpcCtx {
                peer: Box::new(peer.clone()),
            };
            scope.spawn(move|| {
                let mut stream = rf();
                loop {
                    let json = match self.reader.next(&mut stream) {
                        Ok(json) => json,
                        Err(err) => {
                            self.peer.put_rx(Err(err));
                            break
                        }
                    };
                    if json.is_response() {
                        let id = json.get_id().unwrap();
                        match json.into_response() {
                            Ok(resp) => {
                                let resp = resp.map_err(Error::from);
                                self.peer.handle_response(id, resp);
                            }
                            Err(msg) => {
                                print_err!("failed to parse response: {}", msg);
                                self.peer.handle_response(
                                    id, Err(Error::InvalidResponse));
                            }
                        }
                    } else {
                        self.peer.put_rx(Ok(json));
                    }
                }
            });

            loop {
                let read_result = match peer.try_get_rx() {
                    Some(r) => r,
                    None => match peer.try_get_idle() {
                        Some(idle_token) => {
                            handler.idle(&ctx, idle_token);
                            continue;
                        }
                        None => peer.get_rx(),
                    }
                };

                let json = match read_result {
                    Ok(json) => json,
                    Err(err) => {
                        peer.disconnect();
                        return err
                    }
                };

                match json.into_rpc::<H::Notification, H::Request>() {
                    Ok(Call::Request(id, cmd)) => {
                        let result = handler.handle_request(&ctx, cmd);
                        peer.respond(result, id);
                    }
                    Ok(Call::Notification(cmd)) => handler.handle_notification(&ctx, cmd),
                    Ok(Call::InvalidRequest(id, err)) => peer.respond(Err(err), id),
                    Err(err) => {
                        peer.disconnect();
                        return ReadError::UnknownRequest(err)
                    }
                }
            }
        });
        if exit.is_disconnect() {
            Ok(())
        } else {
            Err(exit)
        }
    }
}

impl RpcCtx {
    pub fn get_peer(&self) -> &RpcPeer {
        &self.peer
    }

    /// Schedule the idle handler to be run when there are no requests pending.
    pub fn schedule_idle(&self, token: usize) {
        self.peer.schedule_idle(token)
    }
}

impl<W: Write + Send + 'static> Peer for RawPeer<W> {

    fn box_clone(&self) -> Box<Peer> {
        Box::new((*self).clone())
    }

    fn send_rpc_notification(&self, method: &str, params: &Value) {
        if let Err(e) = self.send(&json!({
            "method": method,
            "params": params,
        })) {
            print_err!("send error on send_rpc_notification method {}: {}",
                       method, e);
        }
    }

    fn send_rpc_request_async(&self, method: &str, params: &Value,
                              f: Box<Callback>) {
        self.send_rpc_request_common(method, params,
                                     ResponseHandler::Callback(f));
    }

    fn send_rpc_request(&self, method: &str, params: &Value)
                        -> Result<Value, Error> {
        let (tx, rx) = mpsc::channel();
        self.send_rpc_request_common(method, params, ResponseHandler::Chan(tx));
        rx.recv().unwrap_or(Err(Error::PeerDisconnect))
    }

    fn request_is_pending(&self) -> bool {
        let queue = self.0.rx_queue.lock().unwrap();
        !queue.is_empty()
    }

    fn schedule_idle(&self, token: usize) {
        self.0.idle_queue.lock().unwrap().push_back(token);
    }

}

impl<W:Write> RawPeer<W> {
    fn send(&self, v: &Value) -> Result<(), io::Error> {
        let mut s = serde_json::to_string(v).unwrap();
        s.push('\n');
        self.0.writer.lock().unwrap().write_all(s.as_bytes())
        // Technically, maybe we should flush here, but doesn't seem to be required.
    }

    fn respond(&self, result: Response, id: u64) {
        let mut response = json!({"id": id});
        match result {
            Ok(result) => response["result"] = result,
            Err(error) => response["error"] = json!(error),
        };
        if let Err(e) = self.send(&response) {
            print_err!("error {} sending response to RPC {:?}", e, id);
        }
    }

    fn send_rpc_request_common(&self, method: &str,
                               params: &Value, rh: ResponseHandler) {
        let id = self.0.id.fetch_add(1, Ordering::Relaxed);
        {
            let mut pending = self.0.pending.lock().unwrap();
            pending.insert(id, rh);
        }
        if let Err(e) = self.send(&json!({
            "id": id,
            "method": method,
            "params": params,
        })) {
            let mut pending = self.0.pending.lock().unwrap();
            if let Some(rh) = pending.remove(&id) {
                rh.invoke(Err(Error::Io(e)));
            }
        }
    }

    fn handle_response(&self, id: u64, resp: Result<Value, Error>) {
        let id = id as usize;
        let handler = {
            let mut pending = self.0.pending.lock().unwrap();
            pending.remove(&id)
        };
        match handler {
            Some(responsehandler) => responsehandler.invoke(resp),
            None => print_err!("id {} not found in pending", id)
        }
    }

    /// Get a message from the receive queue if available.
    fn try_get_rx(&self) -> Option<Result<RpcObject, ReadError>> {
        let mut queue = self.0.rx_queue.lock().unwrap();
        queue.pop_front()
    }

    /// Get a message from the receive queue, blocking until available.
    fn get_rx(&self) -> Result<RpcObject, ReadError> {
        let mut queue = self.0.rx_queue.lock().unwrap();
        while queue.is_empty() {
            queue = self.0.rx_cvar.wait(queue).unwrap();
        }
        queue.pop_front().unwrap()
    }

    /// Adds a message to the receive queue. The message should only
    /// be `None` if the read thread is exiting.
    fn put_rx(&self, json: Result<RpcObject, ReadError>) {
        let mut queue = self.0.rx_queue.lock().unwrap();
        queue.push_back(json);
        self.0.rx_cvar.notify_one();
    }

    fn try_get_idle(&self) -> Option<usize> {
        self.0.idle_queue.lock().unwrap().pop_front()
    }

    /// send disconnect error to pending requests.
    fn disconnect(&self) {
        let mut pending = self.0.pending.lock().unwrap();
        let ids = pending.keys().map(|id| *id).collect::<Vec<_>>();
        for id in ids.iter() {
            let callback = pending.remove(id).unwrap();
            callback.invoke(Err(Error::PeerDisconnect));
        }
    }
}

impl Clone for Box<Peer>
{
    fn clone(&self) -> Box<Peer> {
        self.box_clone()
    }
}

impl<W: Write> Clone for RawPeer<W> {
    fn clone(&self) -> Self {
        RawPeer(self.0.clone())
    }
}

// =============================================================================
//  Helper functions for value manipulation
// =============================================================================

//TODO: delete after finishing migration
pub fn dict_get_u64(dict: &serde_json::Map<String, Value>, key: &str) -> Option<u64> {
    dict.get(key).and_then(Value::as_u64)
}

pub fn dict_get_string<'a>(dict: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    dict.get(key).and_then(Value::as_str)
}

pub fn dict_get_bool<'a>(dict: &'a serde_json::Map<String, Value>, key: &str) -> Option<bool> {
    dict.get(key).and_then(Value::as_bool)
}

pub fn arr_get_u64(arr: &[Value], idx: usize) -> Option<u64> {
    arr.get(idx).and_then(Value::as_u64)
}

pub fn arr_get_i64(arr: &[Value], idx: usize) -> Option<i64> {
    arr.get(idx).and_then(Value::as_i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dict_get_u64() {
        let dict = json!({"life_meaning": 42});
        let dict = dict.as_object().unwrap();
        assert_eq!(dict_get_u64(&dict, "life_meaning"), Some(42));
        assert_eq!(dict_get_u64(&dict, "tea"), None);
    }

    #[test]
    fn test_parse_notif() {
        let reader = MessageReader::default();
        let json = reader.parse(
            r#"{"method": "hi", "params": {"words": "plz"}}"#).unwrap();
        assert!(!json.is_response());
        let rpc = json.into_rpc::<Value, Value>().unwrap();
        match rpc {
            Call::Notification(_) => (),
            _ => panic!("parse failed"),
        }
    }

    #[test]
    fn test_parse_req() {
        let reader = MessageReader::default();
        let json = reader.parse(
            r#"{"id": 5, "method": "hi", "params": {"words": "plz"}}"#).unwrap();
        assert!(!json.is_response());
        let rpc = json.into_rpc::<Value, Value>().unwrap();
        match rpc {
            Call::Request(..) => (),
            _ => panic!("parse failed"),
        }
    }

    #[test]
    fn test_parse_bad_json() {
        // missing "" around params
        let reader = MessageReader::default();
        let json = reader.parse(
            r#"{"id": 5, "method": "hi", params: {"words": "plz"}}"#)
            .err().unwrap();

        match json {
            ReadError::Json(..) => (),
            _ => panic!("parse failed"),
        }
        // not an object
        let json = reader.parse(r#"[5, "hi", {"arg": "val"}]"#).err().unwrap();

        match json {
            ReadError::NotObject => (),
            _ => panic!("parse failed"),
        }
    }
}
