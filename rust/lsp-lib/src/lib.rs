// Copyright 2018 The xi-editor Authors.
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
extern crate serde_derive;
extern crate serde_json;

#[macro_use]
extern crate log;
extern crate chrono;
extern crate fern;

extern crate jsonrpc_lite;
extern crate languageserver_types as lsp_types;
extern crate serde;

extern crate url;
extern crate xi_core_lib as xi_core;
extern crate xi_plugin_lib;
extern crate xi_rope;
extern crate xi_rpc;

use xi_plugin_lib::mainloop;
use xi_plugin_lib::Plugin;

pub mod conversion_utils;
pub mod language_server_client;
pub mod lsp_plugin;
pub mod parse_helper;
mod result_queue;
pub mod types;
mod utils;
pub use crate::lsp_plugin::LspPlugin;
pub use crate::types::Config;

pub fn start_mainloop<P: Plugin>(plugin: &mut P) {
    mainloop(plugin).unwrap();
}
