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

extern crate serde;
extern crate serde_json;
extern crate time;

use std::io;
use std::io::{BufRead, Write};
use serde_json::Value;

#[macro_use]
mod macros;

mod tabs;
mod editor;
mod view;
mod linewrap;

use tabs::Tabs;
use std::io::Error;

extern crate xi_rope;
extern crate xi_unicode;

pub fn send(v: &Value) -> Result<(), Error> {
    let mut s = serde_json::to_string(v).unwrap();
    s.push('\n');
    //print_err!("from core: {}", s);
    io::stdout().write_all(s.as_bytes())
}

fn main() {
    let stdin = io::stdin();
    let mut stdin_handle = stdin.lock();
    let mut buf = String::new();
    let mut tabs = Tabs::new();
    while stdin_handle.read_line(&mut buf).is_ok() {
        if buf.is_empty() {
            break;
        }
        if let Ok(data) = serde_json::from_slice::<Value>(buf.as_bytes()) {
            print_err!("to core: {:?}", data);
            if let Some(req) = data.as_object() {
                if let (Some(method), Some(params)) =
                        (req.get("method").and_then(|v| v.as_string()), req.get("params")) {
                    let id = req.get("id");
                    tabs.handle_rpc(method, params, id);
                }
            }
        }
        buf.clear();
    }
}
