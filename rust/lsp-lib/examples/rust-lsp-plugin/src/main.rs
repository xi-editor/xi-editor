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
use xi_lsp_lib::{start_mainloop, LSPPlugin};

fn main() {
    
    // Assuming RLS is in default path i.e. ~/.cargo/bin/rls
    // TODO: Make this configurable
    let home_dir = std::env::home_dir();
    let mut rls_path = String::from(home_dir.unwrap().to_str().unwrap());
    rls_path.push_str("/.cargo/bin/rls");

    let mut plugin = LSPPlugin::new(
        rls_path,
        vec![],
        vec!["rs".to_string()],
        false,
        Some("Cargo.toml".to_string()),
        "rust".to_string(),
    );

    start_mainloop(&mut plugin);
}
