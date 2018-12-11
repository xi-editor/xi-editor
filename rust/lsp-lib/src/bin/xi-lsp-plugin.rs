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
extern crate xi_lsp_lib;
#[macro_use]
extern crate serde_json;

extern crate chrono;
extern crate fern;
extern crate log;

use xi_lsp_lib::{start_mainloop, Config, LspPlugin};

fn init_logger() -> Result<(), fern::InitError> {
    let level_filter = match std::env::var("XI_LOG") {
        Ok(level) => match level.to_lowercase().as_ref() {
            "trace" => log::LevelFilter::Trace,
            "debug" => log::LevelFilter::Debug,
            _ => log::LevelFilter::Info,
        },
        // Default to info
        Err(_) => log::LevelFilter::Info,
    };

    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}[{}][{}] {}",
                chrono::Local::now().format("[%Y-%m-%d][%H:%M:%S]"),
                record.target(),
                record.level(),
                message
            ))
        })
        .level(level_filter)
        .chain(std::io::stderr())
        .chain(fern::log_file("xi-lsp-plugin.log")?)
        .apply()
        .map_err(|e| e.into())
}

fn main() {
    // The specified language server must be in PATH. XCode does not use
    // the PATH variable of your shell. See the answers below to modify PATH to
    // have language servers in PATH while running from XCode.
    // https://stackoverflow.com/a/17394454 and https://stackoverflow.com/a/43043687
    let config = json!({
        "language_config": {
            // Install instructions here: https://github.com/rust-lang-nursery/rls
            "rust" : {
                "language_name": "Rust",
                "start_command": "rls",
                "start_arguments": [],
                "extensions": ["rs"],
                "supports_single_file": false,
                "workspace_identifier": "Cargo.toml"
            },
            // Install with: npm install -g vscode-json-languageserver
            "json": {
                "language_name": "Json",
                "start_command": "vscode-json-languageserver",
                "start_arguments": ["--stdio"],
                "extensions": ["json", "jsonc"],
                "supports_single_file": true,
            },
            // Install with: npm install -g javascript-typescript-langserver
            "typescript": {
                "language_name": "Typescript",
                "start_command": "javascript-typescript-stdio",
                "start_arguments": [],
                "extensions": ["ts", "js", "jsx", "tsx"],
                "supports_single_file": true,
                "workspace_identifier": "package.json"
            }
        }
    });

    init_logger().expect("Failed to start logger for LSP Plugin");
    let config: Config = serde_json::from_value(config).unwrap();
    let mut plugin = LspPlugin::new(config);

    start_mainloop(&mut plugin);
}
