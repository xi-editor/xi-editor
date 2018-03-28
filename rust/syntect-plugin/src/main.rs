// Copyright 2018 Google Inc. All rights reserved.
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

//! A syntax highlighting plugin based on syntect.

#[macro_use]
extern crate serde_json;

extern crate syntect;
extern crate xi_plugin_lib;
extern crate xi_core_lib as xi_core;
extern crate xi_rope;
extern crate xi_trace;

mod stackmap;
mod local;
mod global;

#[cfg(feature = "global")]
fn main() {
    global::main()
}

#[cfg(not(feature = "global"))]
fn main() {
    local::main()
}
