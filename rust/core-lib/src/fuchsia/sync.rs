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

//! Architecture for synchronizing a CRDT with the ledger. Separated into a
//! module so that it is easier to add other sync stores later.

use magenta::{Channel, ChannelOpts};
use fuchsia::read_entire_vmo;
use super::ledger::{ledger_crash_callback, self};
use apps_ledger_services_public::*;
use std::sync::mpsc::{Sender, Receiver, RecvError};
use std::io::Write;
use fidl::{Promise, Future, self};
use magenta::HandleBase;

use tabs::{BufferIdentifier, BufferContainerRef};
use xi_rope::engine::Engine;
use serde_json;

// TODO switch these to bincode
fn state_to_buf(state: &Engine) -> Vec<u8> {
    serde_json::to_vec(state).unwrap()
}

fn buf_to_state(buf: &[u8]) -> Option<Engine> {
    serde_json::from_slice(buf).ok()
}

/// Stores state needed by the container to perform synchronization.
pub struct SyncStore {
    page: Page_Proxy,
    key: Vec<u8>,
    updates: Sender<SyncMsg>,
    transaction_pending: bool,
    buffer: BufferIdentifier,
}

impl SyncStore {
    /// - `page` is a reference to the Ledger page to store data under.
    /// - `key` is the key the `Syncable` managed by this `SyncStore` will be stored under.
    ///    This example only supports storing things under a single key per page.
    /// - `updates` is a channel to a `SyncUpdater` that will handle events.
    ///
    /// Returns a sync store and schedules the loading of initial
    /// state and subscribes to state updates for this document.
    pub fn new(mut page: Page_Proxy, key: Vec<u8>, updates: Sender<SyncMsg>,
            buffer: BufferIdentifier) -> SyncStore {
        let (s1, s2) = Channel::create(ChannelOpts::Normal).unwrap();
        let watcher_client = PageWatcher_Client::from_handle(s1.into_handle());
        let watcher_client_ptr = ::fidl::InterfacePtr {
            inner: watcher_client,
            version: PageWatcher_Metadata::VERSION,
        };

        let watcher = PageWatcherServer { updates: updates.clone(), buffer: buffer.clone() };
        let _ = fidl::Server::new(watcher, s2).spawn();

        let (mut snap, snap_request) = PageSnapshot_new_pair();
        page.get_snapshot(snap_request, Some(key.clone()), Some(watcher_client_ptr)).with(ledger_crash_callback);

        let initial_state_chan = updates.clone();
        let initial_buffer = buffer.clone();
        snap.get(key.clone()).with(move |raw_res| {
            let res = raw_res.expect("fidl failed on initial state response");
            let value_opt = ledger::value_result(res).expect("failed to read value for key");
            if let Some(buf) = value_opt {
                initial_state_chan.send(SyncMsg::NewState { buffer: initial_buffer, new_buf: buf, done: None }).unwrap();
            }
        });

        SyncStore { page, key, updates, buffer, transaction_pending: false }
    }

    /// Called whenever this app changed its own state and would like to
    /// persist the changes to the ledger. Changes can't be committed
    /// immediately since we have to wait for PageWatcher changes that may not
    /// have arrived yet.
    pub fn state_changed(&mut self) {
        if !self.transaction_pending {
            self.transaction_pending = true;
            let ready_future = self.page.start_transaction();
            let done_chan = self.updates.clone();
            let buffer = self.buffer.clone();
            ready_future.with(move |res| {
                assert_eq!(Status_Ok, res.unwrap(), "failed to start transaction");
                done_chan.send(SyncMsg::TransactionReady { buffer }).unwrap();
            });
        }
    }

    /// Should be called in SyncContainer::transaction_ready to persist the current state.
    pub fn commit_transaction(&mut self, state: &Engine) {
        assert!(self.transaction_pending, "must call state_changed (and wait) before commit");
        self.page.put(self.key.clone(), state_to_buf(state)).with(ledger_crash_callback);
        self.page.commit().with(ledger_crash_callback);
        self.transaction_pending = false;
    }
}

/// All the different asynchronous events the updater thread needs to listen for and act on
pub enum SyncMsg {
    NewState { buffer: BufferIdentifier, new_buf: Vec<u8>, done: Option<Promise<Option<PageSnapshot_Server>, fidl::Error>> },
    TransactionReady { buffer: BufferIdentifier },
    /// Shut down the updater thread
    Stop
}

/// We want to be able to register to recieve events from inside the
/// `SyncStore`/`SyncContainer` but from there we don't have access to the
/// Mutex that holds the container, so we give channel Senders to all the
/// futures so that they can all trigger events in one place that does have
/// the right reference.
///
/// Additionally, the individual `Editor`s aren't wrapped in a `Mutex` so we
/// have to hold a `BufferContainerRef` and use `BufferIdentifier`s with one
/// `SyncUpdater` for all buffers.
pub struct SyncUpdater<W: Write> {
    container_ref: BufferContainerRef<W>,
    chan: Receiver<SyncMsg>,
}

impl<W: Write + Send + 'static> SyncUpdater<W> {
    pub fn new(container_ref: BufferContainerRef<W>, chan: Receiver<SyncMsg>) -> SyncUpdater<W> {
        SyncUpdater { container_ref, chan }
    }

    /// Run this in a thread, it will return when it encounters an error
    /// reading the channel or when the `Stop` message is recieved.
    pub fn work(&self) -> Result<(),RecvError> {
        loop {
            let msg = self.chan.recv()?;
            match msg {
                SyncMsg::Stop => return Ok(()),
                SyncMsg::TransactionReady { buffer }=> {
                    let mut container = self.container_ref.lock();
                    let mut editor = container.editor_for_buffer_mut(&buffer).unwrap();
                    editor.transaction_ready();
                }
                SyncMsg::NewState { new_buf, done, buffer } => {
                    let new = buf_to_state(&new_buf).expect("ledger was set to invalid state");
                    let mut container = self.container_ref.lock();
                    let mut editor = container.editor_for_buffer_mut(&buffer).unwrap();
                    editor.merge_new_state(new);
                    if let Some(promise) = done {
                        promise.set_ok(None);
                    }
                }
            }
        }
    }
}

struct PageWatcherServer {
    updates: Sender<SyncMsg>,
    buffer: BufferIdentifier,
}

impl PageWatcher for PageWatcherServer {
    fn on_change(&mut self, page_change: PageChange, result_state: ResultState) -> Future<Option<PageSnapshot_Server>, fidl::Error> {
        let (future, done) = Future::make_promise();

        assert_eq!(ResultState_Completed, result_state, "example is for single-key pages");
        assert_eq!(page_change.changes.len(), 1, "example is for single-key pages");
        let value_vmo = page_change.changes[0].value.as_ref().expect("example is for single-key pages");
        let new_buf = read_entire_vmo(value_vmo).expect("failed to read key Vmo");
        self.updates.send(SyncMsg::NewState { buffer: self.buffer.clone(), new_buf, done: Some(done) }).unwrap();

        future
    }
}

impl PageWatcher_Stub for PageWatcherServer {
    // Use default dispatching, but we could override it here.
}
impl_fidl_stub!(PageWatcherServer: PageWatcher_Stub);
