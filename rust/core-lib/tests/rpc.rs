// Copyright 2017 The xi-editor Authors.
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

extern crate xi_core_lib;
extern crate xi_rpc;

use std::io;

use xi_core_lib::test_helpers;
use xi_core_lib::XiCore;
use xi_rpc::test_utils::{make_reader, test_channel};
use xi_rpc::{ReadError, RpcLoop};

#[test]
/// Tests that the handler responds to a standard startup sequence as expected.
fn test_startup() {
    let mut state = XiCore::new();
    let (tx, mut rx) = test_channel();
    let mut rpc_looper = RpcLoop::new(tx);
    let json = make_reader(
        r#"{"method":"client_started","params":{}}
{"method":"set_theme","params":{"theme_name":"InspiredGitHub"}}"#,
    );
    assert!(rpc_looper.mainloop(|| json, &mut state).is_ok());
    rx.expect_rpc("available_languages");
    rx.expect_rpc("available_themes");
    rx.expect_rpc("theme_changed");

    let json = make_reader(r#"{"id":0,"method":"new_view","params":{}}"#);
    assert!(rpc_looper.mainloop(|| json, &mut state).is_ok());
    assert_eq!(rx.expect_response(), Ok(json!("view-id-1")));
    rx.expect_rpc("available_plugins");
    rx.expect_rpc("config_changed");
    rx.expect_rpc("language_changed");
    rx.expect_rpc("update");
    rx.expect_rpc("scroll_to");
    rx.expect_nothing();
}

#[test]
/// Tests that the handler creates and destroys views and buffers
fn test_state() {
    let mut state = XiCore::new();

    let write = io::sink();
    let json = make_reader(
        r#"{"method":"client_started","params":{}}
{"id":0,"method":"new_view","params":{"file_path":"../Cargo.toml"}}
{"method":"set_theme","params":{"theme_name":"InspiredGitHub"}}"#,
    );
    let mut rpc_looper = RpcLoop::new(write);
    rpc_looper.mainloop(|| json, &mut state).unwrap();

    {
        let state = state.inner();
        assert_eq!(state._test_open_editors(), vec![test_helpers::new_buffer_id(2)]);
        assert_eq!(state._test_open_views(), vec![test_helpers::new_view_id(1)]);
    }

    let json = make_reader(r#"{"method":"close_view","params":{"view_id":"view-id-1"}}"#);
    rpc_looper.mainloop(|| json, &mut state).unwrap();
    {
        let state = state.inner();
        assert_eq!(state._test_open_views(), Vec::new());
        assert_eq!(state._test_open_editors(), Vec::new());
    }

    let json = make_reader(
        r#"{"id":1,"method":"new_view","params":{}}
{"id":2,"method":"new_view","params":{}}
{"id":3,"method":"new_view","params":{}}"#,
    );

    rpc_looper.mainloop(|| json, &mut state).unwrap();
    {
        let state = state.inner();
        assert_eq!(state._test_open_editors().len(), 3);
    }
}

/// Test whether xi-core invalidates cache lines upon a cursor motion.
#[test]
fn test_invalidate() {
    let mut state = XiCore::new();
    let (tx, mut rx) = test_channel();
    let mut rpc_looper = RpcLoop::new(tx);
    let json = make_reader(
        r#"{"method":"client_started","params":{}}
{"id":0,"method":"new_view","params":{}}
"#,
    );
    assert!(rpc_looper.mainloop(|| json, &mut state).is_ok());

    let mut edit_cmds = String::new();

    for i in 1..20 {
        // add lines "line 1", "line 2",...
        edit_cmds.push_str(r#"{"method":"edit","params":{"view_id":"view-id-1","method":"insert","params":{"chars":"line "#);
        edit_cmds.push_str(&i.to_string());
        edit_cmds.push_str(
            r#""}}}
{"method":"edit","params":{"view_id":"view-id-1","method":"insert_newline","params":[]}}
"#,
        );
    }

    let json = make_reader(edit_cmds);
    assert!(rpc_looper.mainloop(|| json, &mut state).is_ok());

    // jump to line 1, then jump to line 18
    const MOVEMENTS: &str = r#"{"method":"edit","params":{"view_id":"view-id-1","method":"goto_line","params":{"line":1}}}
{"method":"edit","params":{"view_id":"view-id-1","method":"goto_line","params":{"line":18}}}"#;

    let json = make_reader(MOVEMENTS);
    assert!(rpc_looper.mainloop(|| json, &mut state).is_ok());

    let mut last_ops = Vec::new();

    while let Some(Ok(resp)) = rx.next_timeout(std::time::Duration::from_millis(1000)) {
        if !resp.is_response() && resp.get_method().unwrap() == "update" {
            let ops = resp.0.as_object().unwrap()["params"].as_object().unwrap()["update"]
                .as_object()
                .unwrap()["ops"]
                .as_array()
                .unwrap();
            last_ops = ops.clone();

            // Verify that the "invalidate" ops can only go first or last.
            if ops.len() > 2 {
                debug_assert!(
                    ops.iter()
                        // step over leading "invalidate" and "skip"
                        .skip_while(|op| op["op"].as_str().unwrap() == "invalidate"
                            || op["op"].as_str().unwrap() == "skip")
                        // current op (ins/copy/update) adds lines;
                        // wait for another invalidate/skip
                        .skip_while(|op| op["op"].as_str().unwrap() != "invalidate")
                        // step over trailing "invalidate" and "skip"
                        .skip_while(|op| op["op"].as_str().unwrap() == "invalidate"
                            || op["op"].as_str().unwrap() == "skip")
                        .next()
                        .is_none(),
                    "bad update: {}",
                    &ops.iter()
                        .map(|op| format!(
                            "{} {}",
                            op["op"].as_str().unwrap(),
                            op["n"].as_u64().unwrap()
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
    }

    // Dump the last vector of ops.
    // Verify that there is an "update" op in case of a cursor motion.
    assert_eq!(
        last_ops
            .iter()
            .map(|op| {
                let op_in = op.as_object().unwrap();
                (op_in["op"].as_str().unwrap(), op_in["n"].as_u64().unwrap())
            })
            .collect::<Vec<_>>(),
        [("copy", 1), ("update", 1), ("copy", 5), ("copy", 11), ("update", 2)]
    );
}

#[test]
/// Tests that the runloop exits with the correct error when receiving
/// malformed json.
fn test_malformed_json() {
    let mut state = XiCore::new();
    let write = io::sink();
    let mut rpc_looper = RpcLoop::new(write);
    // malformed json: method should be in quotes.
    let read = make_reader(
        r#"{"method":"client_started","params":{}}
{"id":0,method:"new_view","params":{}}"#,
    );
    match rpc_looper.mainloop(|| read, &mut state).err().expect("malformed json exits with error") {
        ReadError::Json(_) => (), // expected
        err => panic!("Unexpected error: {:?}", err),
    }
    // read should have ended after first item
    {
        let state = state.inner();
        assert_eq!(state._test_open_editors().len(), 0);
    }
}

#[test]
/// Sends all of the cursor movement-related commands, and verifies that
/// they are handled.
///
///
/// Note: this is a test of message parsing, not of editor behaviour.
fn test_movement_cmds() {
    let mut state = XiCore::new();
    let write = io::sink();
    let mut rpc_looper = RpcLoop::new(write);
    // init a new view
    let json = make_reader(
        r#"{"method":"client_started","params":{}}
{"method":"set_theme","params":{"theme_name":"InspiredGitHub"}}
{"id":0,"method":"new_view","params":{}}"#,
    );
    assert!(rpc_looper.mainloop(|| json, &mut state).is_ok());

    let json = make_reader(MOVEMENT_RPCS);
    rpc_looper.mainloop(|| json, &mut state).unwrap();
}

#[test]
/// Sends all the commands which modify the buffer, and verifies that they
/// are handled.
fn test_text_commands() {
    let mut state = XiCore::new();
    let write = io::sink();
    let mut rpc_looper = RpcLoop::new(write);
    // init a new view
    let json = make_reader(
        r#"{"method":"client_started","params":{}}
{"method":"set_theme","params":{"theme_name":"InspiredGitHub"}}
{"id":0,"method":"new_view","params":{}}"#,
    );
    assert!(rpc_looper.mainloop(|| json, &mut state).is_ok());

    let json = make_reader(TEXT_EDIT_RPCS);
    rpc_looper.mainloop(|| json, &mut state).unwrap();
}

#[test]
fn test_other_edit_commands() {
    let mut state = XiCore::new();
    let write = io::sink();
    let mut rpc_looper = RpcLoop::new(write);
    // init a new view
    let json = make_reader(
        r#"{"method":"client_started","params":{}}
{"method":"set_theme","params":{"theme_name":"InspiredGitHub"}}
{"id":0,"method":"new_view","params":{}}"#,
    );
    assert!(rpc_looper.mainloop(|| json, &mut state).is_ok());

    let json = make_reader(OTHER_EDIT_RPCS);
    rpc_looper.mainloop(|| json, &mut state).unwrap();
}

#[test]
fn test_settings_commands() {
    let mut state = XiCore::new();
    let (tx, mut rx) = test_channel();
    let mut rpc_looper = RpcLoop::new(tx);
    // init a new view
    let json = make_reader(
        r#"{"method":"client_started","params":{}}
{"method":"set_theme","params":{"theme_name":"InspiredGitHub"}}
{"id":0,"method":"new_view","params":{}}"#,
    );
    assert!(rpc_looper.mainloop(|| json, &mut state).is_ok());
    rx.expect_rpc("available_languages");
    rx.expect_rpc("available_themes");
    rx.expect_rpc("theme_changed");
    rx.expect_response().unwrap();
    rx.expect_rpc("available_plugins");
    rx.expect_rpc("config_changed");
    rx.expect_rpc("language_changed");
    rx.expect_rpc("update");
    rx.expect_rpc("scroll_to");

    let json = make_reader(r#"{"method":"get_config","id":1,"params":{"view_id":"view-id-1"}}"#);
    rpc_looper.mainloop(|| json, &mut state).unwrap();
    let resp = rx.expect_response().unwrap();
    assert_eq!(resp["tab_size"], json!(4));

    let json = make_reader(
        r#"{"method":"modify_user_config","params":{"domain":{"user_override":"view-id-1"},"changes":{"font_face": "Comic Sans"}}}
{"method":"modify_user_config","params":{"domain":{"syntax":"rust"},"changes":{"font_size":42}}}
{"method":"modify_user_config","params":{"domain":"general","changes":{"tab_size":13,"font_face":"Papyrus"}}}"#,
    );
    rpc_looper.mainloop(|| json, &mut state).unwrap();
    // discard config_changed
    rx.expect_rpc("config_changed");
    rx.expect_rpc("update");
    rx.expect_rpc("config_changed");
    rx.expect_rpc("update");

    let json = make_reader(r#"{"method":"get_config","id":2,"params":{"view_id":"view-id-1"}}"#);
    rpc_looper.mainloop(|| json, &mut state).unwrap();
    let resp = rx.expect_response().unwrap();
    assert_eq!(resp["tab_size"], json!(13));
    assert_eq!(resp["font_face"], json!("Comic Sans"));

    // null value should clear entry from this config
    let json = make_reader(
        r#"{"method":"modify_user_config","params":{"domain":{"user_override":"view-id-1"},"changes":{"font_face": null}}}"#,
    );
    rpc_looper.mainloop(|| json, &mut state).unwrap();
    let resp = rx.expect_rpc("config_changed");
    assert_eq!(resp.0["params"]["changes"]["font_face"], json!("Papyrus"));
}

//TODO: test saving rpc
//TODO: test plugin rpc

const MOVEMENT_RPCS: &str = r#"{"method":"edit","params":{"view_id":"view-id-1","method":"move_up","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_down","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_up_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_down_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_left","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_backward","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_right","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_forward","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_left_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_right_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_word_left","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_word_right","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_word_left_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_word_right_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_to_beginning_of_paragraph","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_to_end_of_paragraph","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_to_left_end_of_line","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_to_left_end_of_line_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_to_right_end_of_line","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_to_right_end_of_line_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_to_beginning_of_document","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_to_beginning_of_document_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_to_end_of_document","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"move_to_end_of_document_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"scroll_page_up","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"scroll_page_down","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"page_up_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"page_down_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"select_all","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"add_selection_above","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"add_selection_below","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"collapse_selections","params":[]}}"#;

const TEXT_EDIT_RPCS: &str = r#"{"method":"edit","params":{"view_id":"view-id-1","method":"insert","params":{"chars":"a"}}}
{"method":"edit","params":{"view_id":"view-id-1","method":"delete_backward","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"delete_forward","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"delete_word_forward","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"delete_word_backward","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"delete_to_end_of_paragraph","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"insert_newline","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"insert_tab","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"yank","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"undo","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"redo","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"transpose","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"uppercase","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"lowercase","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"indent","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"outdent","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"duplicate_line","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"replace_next","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"replace_all","params":[]}}
{"id":2,"method":"edit","params":{"view_id":"view-id-1","method":"cut","params":[]}}"#;

const OTHER_EDIT_RPCS: &str = r#"{"method":"edit","params":{"view_id":"view-id-1","method":"scroll","params":[0,1]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"goto_line","params":{"line":1}}}
{"method":"edit","params":{"view_id":"view-id-1","method":"request_lines","params":[0,1]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"drag","params":[17,15,0]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"gesture","params":{"line": 1, "col": 2, "ty": "toggle_sel"}}}
{"method":"edit","params":{"view_id":"view-id-1","method":"gesture","params":{"line": 1, "col": 2, "ty": "point_select"}}}
{"method":"edit","params":{"view_id":"view-id-1","method":"gesture","params":{"line": 1, "col": 2, "ty": "range_select"}}}
{"method":"edit","params":{"view_id":"view-id-1","method":"gesture","params":{"line": 1, "col": 2, "ty": "line_select"}}}
{"method":"edit","params":{"view_id":"view-id-1","method":"gesture","params":{"line": 1, "col": 2, "ty": "word_select"}}}
{"method":"edit","params":{"view_id":"view-id-1","method":"gesture","params":{"line": 1, "col": 2, "ty": "multi_line_select"}}}
{"method":"edit","params":{"view_id":"view-id-1","method":"gesture","params":{"line": 1, "col": 2, "ty": "multi_word_select"}}}
{"method":"edit","params":{"view_id":"view-id-1","method":"find","params":{"case_sensitive":false,"chars":"m"}}}
{"method":"edit","params":{"view_id":"view-id-1","method":"multi_find","params":{"queries": [{"case_sensitive":false,"chars":"m"}]}}}
{"method":"edit","params":{"view_id":"view-id-1","method":"find_next","params":{"wrap_around":true}}}
{"method":"edit","params":{"view_id":"view-id-1","method":"find_previous","params":{"wrap_around":true}}}
{"method":"edit","params":{"view_id":"view-id-1","method":"find_all","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"highlight_find","params":{"visible":true}}}
{"method":"edit","params":{"view_id":"view-id-1","method":"selection_for_find","params":{"case_sensitive":true}}}
{"method":"edit","params":{"view_id":"view-id-1","method":"replace","params":{"chars":"a"}}}
{"method":"edit","params":{"view_id":"view-id-1","method":"selection_for_replace","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"debug_rewrap","params":[]}}
{"method":"edit","params":{"view_id":"view-id-1","method":"debug_print_spans","params":[]}}
{"id":3,"method":"edit","params":{"view_id":"view-id-1","method":"copy","params":[]}}"#;
