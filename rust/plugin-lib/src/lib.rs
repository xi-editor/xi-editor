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

//! The library base for implementing xi-editor plugins.

extern crate xi_core_lib as xi_core;
extern crate xi_rpc;
extern crate xi_rope;
extern crate xi_trace;
extern crate xi_trace_dump;
#[macro_use]
extern crate serde_json;
extern crate serde;
extern crate bytecount;
extern crate rand;

pub mod plugin_base;
pub mod state_cache;
mod base_cache;
