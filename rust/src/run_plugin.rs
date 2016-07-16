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

//! Module to run a plugin.

use std::io::BufReader;
use std::env;
use std::path::PathBuf;
use std::process::{Command,Stdio,ChildStdin};
use std::thread;
use serde_json::Value;

use rpc_peer::{RpcPeer,RpcWriter};

pub type PluginPeer = RpcWriter<ChildStdin>;

pub fn start_plugin<F: 'static + Send + FnOnce(PluginPeer) -> ()>(f: F) {
    thread::spawn(move || {
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
        //print_err!("path = {:?}", pathbuf);
        let mut child = Command::new(&pathbuf)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("plugin failed to start");
        let child_stdin = child.stdin.take().unwrap();
        let child_stdout = child.stdout.take().unwrap();
        let mut peer = RpcPeer::new(BufReader::new(child_stdout), child_stdin);
        let peer_w = peer.get_writer();
        peer_w.send_rpc_async("ping", &Value::Null);
        f(peer_w);
        peer.mainloop(|_method, _params| None);
        let status = child.wait();
        print_err!("child exit = {:?}", status);
    });
}
