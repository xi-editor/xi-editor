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

//! Architecture for synchronizing a CRDT with the ledger. Separated into a
//! module so that it is easier to add other sync stores later.

use std::io::Write;
use std::sync::mpsc::{Receiver, RecvError, Sender};

use log;

use apps_ledger_services_public::*;
use fidl::{self, Future, Promise};
use fuchsia::read_entire_vmo;
use magenta::{Channel, ChannelOpts, HandleBase};
use serde_json;

use super::ledger::{self, ledger_crash_callback};
use tabs::{BufferContainerRef, BufferIdentifier};
use xi_rope::engine::Engine;

// TODO switch these to bincode
fn state_to_buf(state: &Engine) -> Vec<u8> {
    serde_json::to_vec(state).unwrap()
}

fn buf_to_state(buf: &[u8]) -> Result<Engine, serde_json::Error> {
    serde_json::from_slice(buf)
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
    pub fn new(
        mut page: Page_Proxy,
        key: Vec<u8>,
        updates: Sender<SyncMsg>,
        buffer: BufferIdentifier,
    ) -> SyncStore {
        let (s1, s2) = Channel::create(ChannelOpts::Normal).unwrap();
        let watcher_client = PageWatcher_Client::from_handle(s1.into_handle());
        let watcher_client_ptr =
            ::fidl::InterfacePtr { inner: watcher_client, version: PageWatcher_Metadata::VERSION };

        let watcher = PageWatcherServer { updates: updates.clone(), buffer: buffer.clone() };
        let _ = fidl::Server::new(watcher, s2).spawn();

        let (mut snap, snap_request) = PageSnapshot_new_pair();
        page.get_snapshot(snap_request, Some(key.clone()), Some(watcher_client_ptr))
            .with(ledger_crash_callback);

        let initial_state_chan = updates.clone();
        let initial_buffer = buffer.clone();
        snap.get(key.clone()).with(move |raw_res| {
            match raw_res.map(|res| ledger::value_result(res)) {
                Ok(Ok(Some(buf))) => {
                    initial_state_chan
                        .send(SyncMsg::NewState {
                            buffer: initial_buffer,
                            new_buf: buf,
                            done: None,
                        })
                        .unwrap();
                }
                Ok(Ok(None)) => (), // No initial state saved yet
                Err(err) => error!("FIDL failed on initial response: {:?}", err),
                Ok(Err(err)) => error!("Ledger failed to retrieve key: {:?}", err),
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
            ready_future.with(move |res| match res {
                Ok(ledger::OK) => {
                    done_chan.send(SyncMsg::TransactionReady { buffer }).unwrap();
                }
                Ok(err_status) => error!("Ledger failed to start transaction: {:?}", err_status),
                Err(err) => error!("FIDL failed on starting transaction: {:?}", err),
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
    NewState {
        buffer: BufferIdentifier,
        new_buf: Vec<u8>,
        done: Option<Promise<Option<PageSnapshot_Server>, fidl::Error>>,
    },
    TransactionReady {
        buffer: BufferIdentifier,
    },
    /// Shut down the updater thread
    Stop,
}

/// We want to be able to register to receive events from inside the
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
    pub fn work(&self) -> Result<(), RecvError> {
        loop {
            let msg = self.chan.recv()?;
            match msg {
                SyncMsg::Stop => return Ok(()),
                SyncMsg::TransactionReady { buffer } => {
                    let mut container = self.container_ref.lock();
                    // if the buffer was closed, hopefully the page connection was as well, which I hope aborts transactions
                    if let Some(mut editor) = container.editor_for_buffer_mut(&buffer) {
                        editor.transaction_ready();
                    }
                }
                SyncMsg::NewState { new_buf, done, buffer } => {
                    let mut container = self.container_ref.lock();
                    match (container.editor_for_buffer_mut(&buffer), buf_to_state(&new_buf)) {
                        (Some(mut editor), Ok(new_state)) => {
                            editor.merge_new_state(new_state);
                            if let Some(promise) = done {
                                promise.set_ok(None);
                            }
                        }
                        (None, _) => (), // buffer was closed
                        (_, Err(err)) => error!("Ledger was set to invalid state: {:?}", err),
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
    fn on_change(
        &mut self,
        page_change: PageChange,
        result_state: ResultState,
    ) -> Future<Option<PageSnapshot_Server>, fidl::Error> {
        let (future, done) = Future::make_promise();

        let value_opt = page_change.changes.get(0).and_then(|c| c.value.as_ref());
        if let (ledger::RESULT_COMPLETED, Some(value_vmo)) = (result_state, value_opt) {
            let new_buf = read_entire_vmo(value_vmo).expect("failed to read key Vmo");
            self.updates
                .send(SyncMsg::NewState { buffer: self.buffer.clone(), new_buf, done: Some(done) })
                .unwrap();
        } else {
            error!("Xi state corrupted, should have one key but has multiple.");
            // I don't think this should be a FIDL-level error, so set okay
            done.set_ok(None);
        }

        future
    }
}

impl PageWatcher_Stub for PageWatcherServer {
    // Use default dispatching, but we could override it here.
}
impl_fidl_stub!(PageWatcherServer: PageWatcher_Stub);

// ============= Conflict resolution

pub fn start_conflict_resolver_factory(ledger: &mut Ledger_Proxy, key: Vec<u8>) {
    let (s1, s2) = Channel::create(ChannelOpts::Normal).unwrap();
    let resolver_client = ConflictResolverFactory_Client::from_handle(s1.into_handle());
    let resolver_client_ptr = ::fidl::InterfacePtr {
        inner: resolver_client,
        version: ConflictResolverFactory_Metadata::VERSION,
    };

    let _ = fidl::Server::new(ConflictResolverFactoryServer { key }, s2).spawn();

    ledger.set_conflict_resolver_factory(Some(resolver_client_ptr)).with(ledger_crash_callback);
}

struct ConflictResolverFactoryServer {
    key: Vec<u8>,
}

impl ConflictResolverFactory for ConflictResolverFactoryServer {
    fn get_policy(&mut self, _page_id: Vec<u8>) -> Future<MergePolicy, ::fidl::Error> {
        Future::done(Ok(MergePolicy_Custom))
    }

    /// Our resolvers are the same for every page
    fn new_conflict_resolver(&mut self, _page_id: Vec<u8>, resolver: ConflictResolver_Server) {
        let _ = fidl::Server::new(
            ConflictResolverServer { key: self.key.clone() },
            resolver.into_channel(),
        )
        .spawn();
    }
}

impl ConflictResolverFactory_Stub for ConflictResolverFactoryServer {
    // Use default dispatching, but we could override it here.
}
impl_fidl_stub!(ConflictResolverFactoryServer: ConflictResolverFactory_Stub);

fn state_from_snapshot<F>(
    snapshot: ::fidl::InterfacePtr<PageSnapshot_Client>,
    key: Vec<u8>,
    done: F,
) where
    F: Send + FnOnce(Result<Option<Engine>, ()>) + 'static,
{
    assert_eq!(PageSnapshot_Metadata::VERSION, snapshot.version);
    let mut snapshot_proxy = PageSnapshot_new_Proxy(snapshot.inner);
    // TODO get a reference when too big
    snapshot_proxy.get(key).with(move |raw_res| {
        let state = match raw_res.map(|res| ledger::value_result(res)) {
            // the .ok() has the behavior of acting like invalid state is empty
            // and thus deleting invalid state and overwriting it with good state
            Ok(Ok(Some(buf))) => Ok(buf_to_state(&buf).ok()),
            Ok(Ok(None)) => {
                info!("No state in conflicting page");
                Ok(None)
            }
            Err(err) => {
                warn!("FIDL failed on initial response: {:?}", err);
                Err(())
            }
            Ok(Err(err)) => {
                warn!("Ledger failed to retrieve key: {:?}", err);
                Err(())
            }
        };
        done(state);
    });
}

struct ConflictResolverServer {
    key: Vec<u8>,
}

impl ConflictResolver for ConflictResolverServer {
    fn resolve(
        &mut self,
        left: ::fidl::InterfacePtr<PageSnapshot_Client>,
        right: ::fidl::InterfacePtr<PageSnapshot_Client>,
        _common_version: Option<::fidl::InterfacePtr<PageSnapshot_Client>>,
        result_provider: ::fidl::InterfacePtr<MergeResultProvider_Client>,
    ) {
        // TODO in the futures-rs future, do this in parallel with Future combinators
        let key2 = self.key.clone();
        state_from_snapshot(left, self.key.clone(), move |e1_opt| {
            let key3 = key2.clone();
            state_from_snapshot(right, key2, move |e2_opt| {
                let result_opt = match (e1_opt, e2_opt) {
                    (Ok(Some(mut e1)), Ok(Some(e2))) => {
                        e1.merge(&e2);
                        Some(e1)
                    }
                    // one engine didn't exist yet, I'm not sure if Ledger actually generates a conflict in this case
                    (Ok(Some(e)), Ok(None)) | (Ok(None), Ok(Some(e))) => Some(e),
                    // failed to get one of the engines, we can't do the merge properly
                    (Err(()), _) | (_, Err(())) => None,
                    // if state is invalid or missing on both sides, can't merge
                    (Ok(None), Ok(None)) => None,
                };
                if let Some(out_state) = result_opt {
                    let buf = state_to_buf(&out_state);
                    // TODO use a reference here when buf is too big
                    let new_value = Some(Box::new(BytesOrReference::Bytes(buf)));
                    let merged = MergedValue {
                        key: key3,
                        source: ValueSource_New,
                        new_value,
                        priority: Priority_Eager,
                    };
                    assert_eq!(MergeResultProvider_Metadata::VERSION, result_provider.version);
                    let mut result_provider_proxy =
                        MergeResultProvider_new_Proxy(result_provider.inner);
                    result_provider_proxy.merge(vec![merged]);
                    result_provider_proxy.done().with(ledger_crash_callback);
                }
            });
        });
    }
}

impl ConflictResolver_Stub for ConflictResolverServer {
    // Use default dispatching, but we could override it here.
}
impl_fidl_stub!(ConflictResolverServer: ConflictResolver_Stub);
