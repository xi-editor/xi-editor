// Copyright 2017 Google Inc. All rights reserved.
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

#[macro_use]
extern crate serde_json;
extern crate xi_rpc;

use serde_json::Value;
use xi_rpc::{DummyRemote, Handler, RpcCtx, RpcCall, RemoteError};

/// Handler that responds to requests with whatever params they sent.
pub struct EchoHandler;

#[allow(unused)]
impl Handler for EchoHandler {
    type Notification = RpcCall;
    type Request = RpcCall;
    fn handle_notification(&mut self, ctx: RpcCtx, rpc: Self::Notification) {}
    fn handle_request(&mut self, ctx: RpcCtx, rpc: Self::Request)
                      -> Result<Value, RemoteError> {
        Ok(rpc.params)
    }
}

#[test]
fn test_recv_notif() {
    // we should not reply to a well formed notification
    let n = json!({"method": "hullo", "params": {"words": "plz"}});
    let remote = DummyRemote::new(move || EchoHandler);
    let resp = remote.send_notification(&n);
    assert!(resp.is_ok());
    let resp = remote.send_notification(&n);
    assert!(resp.is_ok());
}

#[test]
fn test_recv_resp() {
    // we should reply to a well formed request
    let n = json!({"method": "hullo", "params": {"words": "plz"}});
    let mut remote = DummyRemote::new(move || EchoHandler);
    let resp = remote.send_request(&n).unwrap();
    assert_eq!(resp["words"], json!("plz"));
    // do it again
    let n = json!({"method": "hullo", "params": {"words": "yay"}});
    let resp = remote.send_request(&n).unwrap();
    assert_eq!(resp["words"], json!("yay"));
}

#[test]
fn test_recv_error() {
    // a malformed request containing an ID should receive an error
    let n = json!({
        "method": "hullo",
        "args": {"args": "should", "be": "params"}});
    let mut remote = DummyRemote::new(move || EchoHandler);
    let resp = remote.send_request(&n);
    assert!(resp.is_err(), "{:?}", resp);
}
