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

//! Types and helpers used for testing.

use std::thread;
use std::sync::mpsc::{channel, Sender, Receiver};
use std::time::Duration;
use std::io::{self, BufReader, Read, Write, Cursor};

use serde_json::{self, Value};
use super::{RpcLoop, Handler, RpcCall, RpcCtx, RemoteError};

/// Simulates a remote connection to a Handler.
pub struct DummyRemote {
    tx: Sender<String>,
    rx: Receiver<String>,
    id: u64,
}

struct DummyReader(Receiver<String>, Cursor<String>);
struct DummyWriter(Sender<String>);

impl Read for DummyReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let is_empty = self.1.get_ref().is_empty();
        if is_empty {
            match self.0.recv() {
                Ok(msg) => self.1 = Cursor::new(msg),
                Err(_) => return Ok(0),
            };
        }
        let n = self.1.read(buf);
        if self.1.position() == self.1.get_ref().len() as u64 {
            self.1.get_mut().clear();
        }
        n
    }
}

impl DummyReader {
    fn new(recv: Receiver<String>) -> DummyReader {
        DummyReader(recv, Cursor::new(String::new()))
    }
}

impl Write for DummyWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let s = String::from_utf8(buf.to_vec()).unwrap();
        self.0.send(s)
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "test error"))
            .map(|_| buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl DummyRemote {
    /// Starts a new runloop in a new thread. Messages sent to the runloop
    /// will be passed to the Handler returned from the closure.
    pub fn new<H, HF>(hf: HF) -> Self
    where H: Handler,
          HF: 'static + Send + FnOnce() -> H
    {
        let (local_tx, remote_rx) = channel();
        let (remote_tx, local_rx) = channel();

        thread::spawn(move || {
            let mut looper = RpcLoop::new(DummyWriter(remote_tx));
            let mut handler = hf();
            let reader = DummyReader::new(remote_rx);
            looper.mainloop(move || BufReader::new(reader), &mut handler);
        });

        DummyRemote {
            tx: local_tx,
            rx: local_rx,
            id: 0,
        }
    }

    /// Sends a message, and blocks with a reasonable timeout on the response.
    fn send_common(&self, v: &Value) -> Option<String> {
        let mut s = serde_json::to_string(v).unwrap();
        s.push('\n');
        self.tx.send(s).unwrap();
        match self.rx.recv_timeout(Duration::from_millis(500)) {
            Ok(msg) => Some(msg),
            Err(_) => None
        }
    }

    /// Sends a notification and checks for a response. If a response is received,
    /// it is returned in the error.
    pub fn send_notification(&self, v: &Value) -> Result<(), String> {
        match self.send_common(v) {
            None => Ok(()),
            Some(msg) => Err(msg)
        }
    }

    /// Sends a request and waits for a response. If none is received, returns
    /// an error.
    pub fn send_request(&mut self, v: &Value) -> Result<String, ()> {
        let mut v = v.to_owned();
        v["id"] = json!(self.id);
        self.id += 1;
        match self.send_common(&v) {
            None => Err(()),
            Some(msg) => Ok(msg),
        }
    }
}

/// Handler that responds to requests with whatever params they sent.
pub struct EchoHandler;

#[allow(unused)]
impl Handler for EchoHandler {
    type Notification = RpcCall;
    type Request = RpcCall;
    fn handle_notification(&mut self, ctx: RpcCtx, rpc: Self::Notification) {
        // chill
    }

    fn handle_request(&mut self, ctx: RpcCtx, rpc: Self::Request) ->
        Result<Value, RemoteError> {
            return Ok(rpc.params)
    }
}
