// Copyright 2016 The xi-editor Authors.
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

//! A simple test program for evaluating the speed of cross-thread communications.
extern crate xi_rpc;

use std::sync::mpsc;
use std::thread;

/*
use xi_rpc::chan::Chan;

pub fn test_chan() {
    let n_iter = 1000000;
    let chan1 = Chan::new();
    let chan1s = chan1.clone();
    let chan2 = Chan::new();
    let chan2s = chan2.clone();
    let thread1 = thread::spawn(move|| {
        for _ in 0..n_iter {
            chan2s.try_send(chan1.recv());
        }
    });
    let thread2 = thread::spawn(move|| {
        for _ in 0..n_iter {
            chan1s.try_send(42);
            let _ = chan2.recv();
        }
    });
    let _ = thread1.join();
    let _ = thread2.join();
}
*/

pub fn test_mpsc() {
    let n_iter = 1000000;
    let (chan1s, chan1) = mpsc::channel();
    let (chan2s, chan2) = mpsc::channel();
    let thread1 = thread::spawn(move || {
        for _ in 0..n_iter {
            chan2s.send(chan1.recv()).unwrap();
        }
    });
    let thread2 = thread::spawn(move || {
        for _ in 0..n_iter {
            chan1s.send(42).unwrap();
            let _ = chan2.recv();
        }
    });
    let _ = thread1.join();
    let _ = thread2.join();
}

pub fn main() {
    test_mpsc()
}
