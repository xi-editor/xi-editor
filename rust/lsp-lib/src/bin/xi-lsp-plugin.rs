// Copyright 2018 Google LLC
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

extern crate xi_lsp_lib;
#[macro_use]
extern crate serde_json;

use serde_json::{Value};
use xi_lsp_lib::{start_mainloop, LSPPlugin, Config};

fn main() {

    let config = json!({
        "language_config": {
            "rust" : {
                "language_name": "Rust",
                "start_command": "/Users/betterclever/.cargo/bin/rls",
                "start_arguments": [],
                "extensions": ["rs"],
                "supports_single_file": false,
                "workspace_identifier": "Cargo.toml"
            },
            "json": {
                "language_name": "Json",
                "start_command": "/usr/local/bin/vscode-json-languageserver",
                "start_arguments": ["--stdio"],
                "extensions": ["json", "jsonc"],
                "supports_single_file": true,
            }
        }
    });

    let config: Config = serde_json::from_value(config).unwrap();
    let mut plugin = LSPPlugin::new(config);

    start_mainloop(&mut plugin);
}
