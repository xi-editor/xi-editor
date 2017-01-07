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

use std::io;

#[macro_use]
mod macros;

extern crate xi_core_lib;
extern crate xi_rpc;

use xi_core_lib::MainState;

use xi_rpc::RpcLoop;

fn main() {
    let mut state = MainState::new();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut rpc_looper = RpcLoop::new(stdout);

    rpc_looper.mainloop(|| stdin.lock(), &mut state);
}
