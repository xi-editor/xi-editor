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

use notify::{Watcher, RecursiveMode, watcher, DebouncedEvent};
use std::path::Path;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::thread;
use std::mem;
use std::collections::VecDeque;

use xi_rpc::RpcPeer;

/// Delay for aggregating related file system events.
pub const DEBOUNCE_WAIT_MILLIS: u64 = 50;

/// Token provided to `FsWatcher`, to associate events with registrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventToken(pub usize);

/// Wrapper around `notify::Watcher`, adding support for the xi_rpc runloop.
#[derive(Debug, Clone, Default)]
pub struct FsWatcher {
    pub events: Arc<Mutex<VecDeque<(EventToken, DebouncedEvent)>>>,
}

impl FsWatcher {
    /// Begin watching `path`. As `DebouncedEvent`s (documented in the [notify](https://docs.rs/notify/4.0.2/notify/) crate)
    /// arrive, they are stored with the associated `token` and a task is
    /// added to the runloop's idle queue.
    ///
    /// Delivery of events then requires that the runloop's handler
    /// correctly forward the `handle_idle` call to the interested party.
    pub fn watch<P>(&mut self, path: P, recursive_mode: RecursiveMode,
                token: EventToken, peer: &RpcPeer)
        where P: AsRef<Path>,
    {
        self.watch_filtered(path, recursive_mode, token, peer, |_| { true });
    }

    /// Like `watch`, but taking a predicate function that filters delivery
    /// of events based on their path.
    pub fn watch_filtered<P, F>(&mut self, path: P, recursive_mode: RecursiveMode,
                                token: EventToken, peer: &RpcPeer, predicate: F)
        where P: AsRef<Path>,
              F: Fn(&Path) -> bool + Send + 'static,
    {
        let path = path.as_ref().to_owned();
        let peer = peer.clone();
        let events = self.events.clone();
        thread::spawn(move || {

            let (tx, rx) = channel();
            let mut watcher = watcher(tx, Duration::from_millis(DEBOUNCE_WAIT_MILLIS))
                .unwrap();

            watcher.watch(&path, recursive_mode).unwrap();

            loop {
                match rx.recv() {
                    Ok(event) =>  {
                        if apply_filter(&predicate, &event) {
                            events.lock().unwrap().push_back((token, event));
                            peer.schedule_idle(::tabs::WATCH_IDLE_TOKEN);
                        }
                    },
                    Err(e) => {
                        //TODO: how do we handle unexpected disconnects?
                        eprintln!("watcher returned error {:?} for path {:?}, \
                        token {:?}", e, &path, token);
                        break
                    }
                }
            }
        });
    }

    /// Takes ownership of this `Watcher`'s current event queue.
    pub fn take_events(&mut self) -> VecDeque<(EventToken, DebouncedEvent)> {
        let mut events = self.events.lock().unwrap();
        mem::replace(&mut events, VecDeque::new())
    }

    //TODO impl unwatch, when we add in watching of opened files
}

/// Checks the predicate against the various event cases
fn apply_filter<F>(filter: &F, event: &DebouncedEvent) -> bool
    where F: Fn(&Path) -> bool,
{
    use self::DebouncedEvent::*;
    match *event {
        NoticeWrite(ref p) | NoticeRemove(ref p) | Create(ref p) |
            Write(ref p) | Chmod(ref p) | Remove(ref p) => {
                filter(p)
            }
        Rename(ref p1, ref p2) => {
            filter(p1) || filter(p2)
        }
        Rescan => false,
        Error(_, ref opt_p) => {
            opt_p.as_ref().map(|p| filter(p))
                .unwrap_or(false)
        }
    }
}

