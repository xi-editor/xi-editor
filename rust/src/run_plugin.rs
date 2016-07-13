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

use std::env;
use std::path::PathBuf;
use std::process::Command;

pub fn start_plugin() {
    let path = match env::args_os().next() {
        Some(path) => path,
        _ => {
            print_err!("empty args, that's strange");
            return;
        }
    };
    let mut pathbuf = PathBuf::from(&path);
    pathbuf.pop();
    pathbuf.push("python");
    pathbuf.push("plugin.py");
    print_err!("path = {:?}", pathbuf);
    Command::new(&pathbuf)
        .spawn()
        .expect("plugin failed to start");
}
