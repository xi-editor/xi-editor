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
extern crate xi_core_lib;

use std::io;

use xi_rpc::{RpcLoop, ReadError};
use xi_rpc::test_utils::{make_reader, test_channel};
use xi_core_lib::MainState;

#[test]
fn test_startup() {
    let mut state = MainState::new();
    let (tx, mut rx) = test_channel();
    let mut rpc_looper = RpcLoop::new(tx);
    let json = make_reader(r#"{"method":"client_started","params":{}}
{"method":"set_theme","params":{"theme_name":"InspiredGitHub"}}"#);
    assert!(rpc_looper.mainloop(|| json, &mut state).is_ok());
    assert_eq!(rx.expect_object().get_method(), Some("available_themes"));
    assert_eq!(rx.expect_object().get_method(), Some("theme_changed"));

    let json = make_reader(r#"{"id":0,"method":"new_view","params":{}}"#);
    assert!(rpc_looper.mainloop(|| json, &mut state).is_ok());
    assert_eq!(rx.expect_response(), Ok(json!("view-id-1")));
}


#[test]
fn test_state() {
    let mut state = MainState::new();
    let buffers = state._get_buffers();

    let write = io::sink();
    let json = make_reader(r#"{"method":"client_started","params":{}}
{"method":"set_theme","params":{"theme_name":"InspiredGitHub"}}
{"id":0,"method":"new_view","params":{}}"#);
    let mut rpc_looper = RpcLoop::new(write);
    assert!(rpc_looper.mainloop(|| json, &mut state).is_ok());

    {
        let buffers = buffers.lock();
        assert_eq!(buffers.iter_editors().count(), 1);
    }
    assert!(buffers.buffer_for_view(&"view-id-1".into()).is_some());

    let json = make_reader(
        r#"{"method":"close_view","params":{"view_id":"view-id-1"}}"#);
    assert!(rpc_looper.mainloop(|| json, &mut state).is_ok());
    {
        let buffers = buffers.lock();
        assert_eq!(buffers.iter_editors().count(), 0);
    }

    let json = make_reader(r#"{"id":1,"method":"new_view","params":{}}
{"id":2,"method":"new_view","params":{}}
{"id":3,"method":"new_view","params":{}}"#);

    assert!(rpc_looper.mainloop(|| json, &mut state).is_ok());
    {
        let buffers = buffers.lock();
        assert_eq!(buffers.iter_editors().count(), 3);
    }
}

#[test]
fn test_malformed_json() {
    let mut state = MainState::new();
    let write = io::sink();
    let mut rpc_looper = RpcLoop::new(write);
    // malformed json, no id: should not receive a response, and connection should close.
    let read = make_reader(r#"{method:"client_started","params":{}}
{"id":0,"method":"new_view","params":{}}"#);
    match rpc_looper.mainloop(|| read, &mut state).err()
        .expect("malformed json exits with error") {
            ReadError::Json(_) => (), // expected
            err => panic!("Unexpected error: {:?}", err),
    }
    // read should have ended after first item
    {
        let buffers = state._get_buffers();
        let buffers = buffers.lock();
        assert_eq!(buffers.iter_editors().count(), 0);
    }
}
