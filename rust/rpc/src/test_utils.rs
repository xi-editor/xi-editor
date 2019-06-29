// Copyright 2017 The xi-editor Authors.
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

use std::io::{self, Cursor, Write};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use serde_json::{self, Value};

use super::{Callback, Error, MessageReader, Peer, ReadError, Response, RpcObject};

/// Wraps an instance of `mpsc::Sender`, implementing `Write`.
///
/// This lets the tx side of an mpsc::channel serve as the destination
/// stream for an RPC loop.
pub struct DummyWriter(Sender<String>);

/// Wraps an instance of `mpsc::Receiver`, providing convenience methods
/// for parsing received messages.
pub struct DummyReader(MessageReader, Receiver<String>);

/// An Peer that doesn't do anything.
#[derive(Debug, Clone)]
pub struct DummyPeer;

/// Returns a `(DummyWriter, DummyReader)` pair.
pub fn test_channel() -> (DummyWriter, DummyReader) {
    let (tx, rx) = channel();
    (DummyWriter(tx), DummyReader(MessageReader::default(), rx))
}

/// Given a string type, returns a `Cursor<Vec<u8>>`, which implements
/// `BufRead`.
pub fn make_reader<S: AsRef<str>>(s: S) -> Cursor<Vec<u8>> {
    Cursor::new(s.as_ref().as_bytes().to_vec())
}

impl DummyReader {
    /// Attempts to read a message, returning `None` if the wait exceeds
    /// `timeout`.
    ///
    /// This method makes no assumptions about the contents of the
    /// message, and does no error handling.
    pub fn next_timeout(&mut self, timeout: Duration) -> Option<Result<RpcObject, ReadError>> {
        self.1.recv_timeout(timeout).ok().map(|s| self.0.parse(&s))
    }

    /// Reads and parses a response object.
    ///
    /// # Panics
    ///
    /// Panics if a non-response message is received, or if no message
    /// is received after a reasonable time.
    pub fn expect_response(&mut self) -> Response {
        let raw = self.next_timeout(Duration::from_secs(1)).expect("response should be received");
        let val = raw.as_ref().ok().map(|v| serde_json::to_string(&v.0));
        let resp = raw.map_err(|e| e.to_string()).and_then(|r| r.into_response());

        match resp {
            Err(msg) => panic!("Bad response: {:?}. {}", val, msg),
            Ok(resp) => resp,
        }
    }

    pub fn expect_object(&mut self) -> RpcObject {
        self.next_timeout(Duration::from_secs(1)).expect("expected object").unwrap()
    }

    pub fn expect_rpc(&mut self, method: &str) -> RpcObject {
        let obj = self
            .next_timeout(Duration::from_secs(1))
            .unwrap_or_else(|| panic!("expected rpc \"{}\"", method))
            .unwrap();
        assert_eq!(obj.get_method(), Some(method));
        obj
    }

    pub fn expect_nothing(&mut self) {
        if let Some(thing) = self.next_timeout(Duration::from_millis(500)) {
            panic!("unexpected something {:?}", thing);
        }
    }
}

impl Write for DummyWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let s = String::from_utf8(buf.to_vec()).unwrap();
        self.0
            .send(s)
            .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("{:?}", err)))
            .map(|_| buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Peer for DummyPeer {
    fn box_clone(&self) -> Box<dyn Peer> {
        Box::new(self.clone())
    }
    fn send_rpc_notification(&self, _method: &str, _params: &Value) {}
    fn send_rpc_request_async(&self, _method: &str, _params: &Value, f: Box<dyn Callback>) {
        f.call(Ok("dummy peer".into()))
    }
    fn send_rpc_request(&self, _method: &str, _params: &Value) -> Result<Value, Error> {
        Ok("dummy peer".into())
    }
    fn request_is_pending(&self) -> bool {
        false
    }
    fn schedule_idle(&self, _token: usize) {}
    fn schedule_timer(&self, _time: Instant, _token: usize) {}
}
