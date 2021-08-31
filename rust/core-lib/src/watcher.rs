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

//! Monitoring files and directories.
//!
//! This module contains `FileWatcher` and related types, responsible for
//! monitoring changes to files and directories. Under the hood it is a
//! thin wrapper around some concrete type provided by the
//! [`notify`](https://docs.rs/notify) crate; the implementation is
//! platform dependent, and may be using kqueue, fsevent, or another
//! low-level monitoring system.
//!
//! Our wrapper provides a few useful features:
//!
//! - All `watch` calls are associated with a `WatchToken`; this
//! allows for the same path to be watched multiple times,
//! presumably by multiple interested parties. events are delivered
//! once-per token.
//!
//! - There is the option (via `FileWatcher::watch_filtered`) to include
//! a predicate along with a path, to filter paths before delivery.
//!
//! - We are integrated with the xi_rpc runloop; events are queued as
//! they arrive, and an idle task is scheduled.

use crossbeam_channel::unbounded;
use notify::{event::*, watcher, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::VecDeque;
use std::fmt;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use xi_rpc::RpcPeer;

/// Delay for aggregating related file system events.
pub const DEBOUNCE_WAIT_MILLIS: u64 = 50;

/// Wrapper around a `notify::Watcher`. It runs the inner watcher
/// in a separate thread, and communicates with it via a [crossbeam channel].
/// [crossbeam channel]: https://docs.rs/crossbeam-channel
pub struct FileWatcher {
    inner: RecommendedWatcher,
    state: Arc<Mutex<WatcherState>>,
}

#[derive(Debug, Default)]
struct WatcherState {
    events: EventQueue,
    watchees: Vec<Watchee>,
}

/// Tracks a registered 'that-which-is-watched'.
#[doc(hidden)]
struct Watchee {
    path: PathBuf,
    recursive: bool,
    token: WatchToken,
    filter: Option<Box<PathFilter>>,
}

/// Token provided to `FileWatcher`, to associate events with
/// interested parties.
///
/// Note: `WatchToken`s are assumed to correspond with an
/// 'area of interest'; that is, they are used to route delivery
/// of events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchToken(pub usize);

/// A trait for types which can be notified of new events.
/// New events are accessible through the `FileWatcher` instance.
pub trait Notify: Send {
    fn notify(&self);
}

pub type EventQueue = VecDeque<(WatchToken, Event)>;

pub type PathFilter = dyn Fn(&Path) -> bool + Send + 'static;

impl FileWatcher {
    pub fn new<T: Notify + 'static>(peer: T) -> Self {
        let (tx_event, rx_event) = unbounded();

        let state = Arc::new(Mutex::new(WatcherState::default()));
        let state_clone = state.clone();

        let inner = watcher(tx_event, Duration::from_millis(100)).expect("watcher should spawn");

        thread::spawn(move || {
            while let Ok(Ok(event)) = rx_event.recv() {
                let mut state = state_clone.lock().unwrap();
                let WatcherState { ref mut events, ref mut watchees } = *state;

                watchees
                    .iter()
                    .filter(|w| w.wants_event(&event))
                    .map(|w| w.token)
                    .for_each(|t| events.push_back((t, event.clone())));

                peer.notify();
            }
        });

        FileWatcher { inner, state }
    }

    /// Begin watching `path`. As `Event`s (documented in the
    /// [notify](https://docs.rs/notify) crate) arrive, they are stored
    /// with the associated `token` and a task is added to the runloop's
    /// idle queue.
    ///
    /// Delivery of events then requires that the runloop's handler
    /// correctly forward the `handle_idle` call to the interested party.
    pub fn watch(&mut self, path: &Path, recursive: bool, token: WatchToken) {
        self.watch_impl(path, recursive, token, None);
    }

    /// Like `watch`, but taking a predicate function that filters delivery
    /// of events based on their path.
    pub fn watch_filtered<F>(&mut self, path: &Path, recursive: bool, token: WatchToken, filter: F)
    where
        F: Fn(&Path) -> bool + Send + 'static,
    {
        let filter = Box::new(filter) as Box<PathFilter>;
        self.watch_impl(path, recursive, token, Some(filter));
    }

    fn watch_impl(
        &mut self,
        path: &Path,
        recursive: bool,
        token: WatchToken,
        filter: Option<Box<PathFilter>>,
    ) {
        let path = match path.canonicalize() {
            Ok(ref p) => p.to_owned(),
            Err(e) => {
                warn!("error watching {:?}: {:?}", path, e);
                return;
            }
        };

        let mut state = self.state.lock().unwrap();

        let w = Watchee { path, recursive, token, filter };
        let mode = mode_from_bool(w.recursive);

        if !state.watchees.iter().any(|w2| w.path == w2.path) {
            if let Err(e) = self.inner.watch(&w.path, mode) {
                warn!("watching error {:?}", e);
            }
        }

        state.watchees.push(w);
    }

    /// Removes the provided token/path pair from the watch list.
    /// Does not stop watching this path, if it is associated with
    /// other tokens.
    pub fn unwatch(&mut self, path: &Path, token: WatchToken) {
        let mut state = self.state.lock().unwrap();

        let idx = state.watchees.iter().position(|w| w.token == token && w.path == path);

        if let Some(idx) = idx {
            let removed = state.watchees.remove(idx);
            if !state.watchees.iter().any(|w| w.path == removed.path) {
                if let Err(e) = self.inner.unwatch(&removed.path) {
                    warn!("unwatching error {:?}", e);
                }
            }
            //TODO: Ideally we would be tracking what paths we're watching with
            // some prefix-tree-like structure, which would let us keep track
            // of when some child path might need to be reregistered. How this
            // works and when registration would be required is dependent on
            // the underlying notification mechanism, however. There's an
            // in-progress rewrite of the Notify crate which use under the
            // hood, and a component of that rewrite is adding this
            // functionality; so until that lands we're using a fairly coarse
            // heuristic to determine if we need to re-watch subpaths.

            // if this was recursive, check if any child paths need to be
            // manually re-added
            if removed.recursive {
                // do this in two steps because we've borrowed mutably up top
                let to_add = state
                    .watchees
                    .iter()
                    .filter(|w| w.path.starts_with(&removed.path))
                    .map(|w| (w.path.to_owned(), mode_from_bool(w.recursive)))
                    .collect::<Vec<_>>();

                for (path, mode) in to_add {
                    if let Err(e) = self.inner.watch(&path, mode) {
                        warn!("watching error {:?}", e);
                    }
                }
            }
        }
    }

    /// Takes ownership of this `Watcher`'s current event queue.
    pub fn take_events(&mut self) -> VecDeque<(WatchToken, Event)> {
        let mut state = self.state.lock().unwrap();
        let WatcherState { ref mut events, .. } = *state;
        mem::take(events)
    }
}

impl Watchee {
    fn wants_event(&self, event: &Event) -> bool {
        match &event.kind {
            EventKind::Create(CreateKind::Any)
            | EventKind::Remove(RemoveKind::Any)
            | EventKind::Modify(ModifyKind::Any)
            | EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)) => {
                if event.paths.len() == 1 {
                    self.applies_to_path(&event.paths[0])
                } else {
                    info!(
                        "Rejecting event {:?} with incorrect paths. Expected 1 found {}.",
                        event,
                        event.paths.len()
                    );
                    false
                }
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
                if event.paths.len() == 2 {
                    //There will be two paths. First is "from" and other is "to".
                    self.applies_to_path(&event.paths[0]) || self.applies_to_path(&event.paths[1])
                } else {
                    info!(
                        "Rejecting event {:?} with incorrect paths. Expected 2 found {}.",
                        event,
                        event.paths.len()
                    );
                    false
                }
            }
            _ => false,
        }
    }

    fn applies_to_path(&self, path: &Path) -> bool {
        let general_case = if path.starts_with(&self.path) {
            (self.recursive || self.path == path) || path.parent() == Some(&self.path)
        } else {
            false
        };

        if let Some(ref filter) = self.filter {
            general_case && filter(path)
        } else {
            general_case
        }
    }
}

impl Notify for RpcPeer {
    fn notify(&self) {
        self.schedule_idle(crate::tabs::WATCH_IDLE_TOKEN);
    }
}

impl fmt::Debug for Watchee {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Watchee path: {:?}, r {}, t {} f {}",
            self.path,
            self.recursive,
            self.token.0,
            self.filter.is_some()
        )
    }
}

fn mode_from_bool(is_recursive: bool) -> RecursiveMode {
    if is_recursive {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    }
}

#[cfg(test)]
extern crate tempdir;

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use notify::EventKind;
    use std::ffi::OsStr;
    use std::fs;
    use std::io::Write;
    use std::thread;
    use std::time::{Duration, Instant};

    impl PartialEq<usize> for WatchToken {
        fn eq(&self, other: &usize) -> bool {
            self.0 == *other
        }
    }

    impl From<usize> for WatchToken {
        fn from(err: usize) -> WatchToken {
            WatchToken(err)
        }
    }

    impl Notify for crossbeam_channel::Sender<bool> {
        fn notify(&self) {
            self.send(true).expect("send shouldn't fail")
        }
    }

    // Sleep for `duration` in milliseconds
    pub fn sleep(millis: u64) {
        thread::sleep(Duration::from_millis(millis));
    }

    // Sleep for `duration` in milliseconds if running on OS X
    pub fn sleep_if_macos(millis: u64) {
        if cfg!(target_os = "macos") {
            sleep(millis)
        }
    }

    pub fn recv_all<T>(rx: &crossbeam_channel::Receiver<T>, duration: Duration) -> Vec<T> {
        let start = Instant::now();
        let mut events = Vec::new();

        while start.elapsed() < duration {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(event) => events.push(event),
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => (),
                Err(e) => panic!("unexpected channel err: {:?}", e),
            }
        }
        events
    }

    // from https://github.com/passcod/notify/blob/master/tests/utils/mod.rs
    pub trait TestHelpers {
        /// Return path relative to the TempDir. Directory separator must
        /// be a forward slash, and will be converted to the platform's
        /// native separator.
        fn mkpath(&self, p: &str) -> PathBuf;
        /// Create file or directory. Directories must contain the phrase
        /// "dir" otherwise they will be interpreted as files.
        fn create(&self, p: &str);
        /// Create all files and directories in the `paths` list.
        /// Directories must contain the phrase "dir" otherwise they
        /// will be interpreted as files.
        fn create_all(&self, paths: Vec<&str>);
        /// Rename file or directory.
        fn rename(&self, a: &str, b: &str);
        ///// Toggle "other" rights on linux and os x and "readonly" on windows
        //fn chmod(&self, p: &str);
        /// Write some data to a file
        fn write(&self, p: &str);
        /// Remove file or directory
        fn remove(&self, p: &str);
    }

    impl TestHelpers for tempdir::TempDir {
        fn mkpath(&self, p: &str) -> PathBuf {
            let mut path = self.path().canonicalize().expect("failed to canonalize path");
            for part in p.split('/').collect::<Vec<_>>() {
                if part != "." {
                    path.push(part);
                }
            }
            path
        }

        fn create(&self, p: &str) {
            let path = self.mkpath(p);
            if path.components().last().unwrap().as_os_str().to_str().unwrap().contains("dir") {
                fs::create_dir_all(path).expect("failed to create directory");
            } else {
                let parent = path.parent().expect("failed to get parent directory").to_owned();
                if !parent.exists() {
                    fs::create_dir_all(parent).expect("failed to create parent directory");
                }
                fs::File::create(path).expect("failed to create file");
            }
        }

        fn create_all(&self, paths: Vec<&str>) {
            for p in paths {
                self.create(p);
            }
        }

        fn rename(&self, a: &str, b: &str) {
            let path_a = self.mkpath(a);
            let path_b = self.mkpath(b);
            fs::rename(&path_a, &path_b).expect("failed to rename file or directory");
        }

        fn write(&self, p: &str) {
            let path = self.mkpath(p);

            let mut file =
                fs::OpenOptions::new().write(true).open(path).expect("failed to open file");

            file.write_all(b"some data").expect("failed to write to file");
            file.sync_all().expect("failed to sync file");
        }

        fn remove(&self, p: &str) {
            let path = self.mkpath(p);
            if path.is_dir() {
                fs::remove_dir(path).expect("failed to remove directory");
            } else {
                fs::remove_file(path).expect("failed to remove file");
            }
        }
    }

    #[test]
    fn test_applies_to_path() {
        let mut w = Watchee {
            path: PathBuf::from("/hi/there/"),
            recursive: false,
            token: WatchToken(1),
            filter: None,
        };
        assert!(w.applies_to_path(&PathBuf::from("/hi/there/friend.txt")));
        assert!(w.applies_to_path(&PathBuf::from("/hi/there/")));
        assert!(!w.applies_to_path(&PathBuf::from("/hi/there/dear/friend.txt")));
        assert!(!w.applies_to_path(&PathBuf::from("/oh/hi/there/")));

        w.recursive = true;
        assert!(w.applies_to_path(&PathBuf::from("/hi/there/dear/friend.txt")));
        assert!(w.applies_to_path(&PathBuf::from("/hi/there/friend.txt")));
        assert!(w.applies_to_path(&PathBuf::from("/hi/there/")));

        w.filter = Some(Box::new(|p| p.extension().and_then(OsStr::to_str) == Some("txt")));
        assert!(w.applies_to_path(&PathBuf::from("/hi/there/dear/friend.txt")));
        assert!(w.applies_to_path(&PathBuf::from("/hi/there/friend.txt")));
        assert!(!w.applies_to_path(&PathBuf::from("/hi/there/")));
        assert!(!w.applies_to_path(&PathBuf::from("/hi/there/friend.exe")));
        assert!(w.applies_to_path(&PathBuf::from("/hi/there/my/old/sweet/pal.txt")));
    }

    //https://github.com/passcod/notify/issues/131
    #[test]
    #[cfg(unix)]
    fn test_crash_repro() {
        let (tx, _rx) = unbounded();
        let path = PathBuf::from("/bin/cat");
        let mut w = watcher(tx, Duration::from_secs(1)).unwrap();
        w.watch(&path, RecursiveMode::NonRecursive).unwrap();
        sleep(20);
        w.watch(&path, RecursiveMode::NonRecursive).unwrap();
        w.unwatch(&path).unwrap();
    }

    #[test]
    fn recurse_with_contained() {
        let (tx, rx) = unbounded();
        let tmp = tempdir::TempDir::new("xi-test-recurse-contained").unwrap();
        let mut w = FileWatcher::new(tx);
        tmp.create("adir/dir2/file");
        sleep_if_macos(35_000);
        w.watch(&tmp.mkpath("adir"), true, 1.into());
        sleep(10);
        w.watch(&tmp.mkpath("adir/dir2/file"), false, 2.into());
        sleep(10);
        w.unwatch(&tmp.mkpath("adir"), 1.into());
        sleep(10);
        tmp.write("adir/dir2/file");
        let _ = recv_all(&rx, Duration::from_millis(1000));
        let events = w.take_events();
        assert_eq!(
            events,
            vec![
                (
                    2.into(),
                    Event::new(EventKind::Modify(ModifyKind::Any))
                        .add_path(tmp.mkpath("adir/dir2/file"))
                        .set_flag(Flag::Notice)
                ),
                (
                    2.into(),
                    Event::new(EventKind::Modify(ModifyKind::Any))
                        .add_path(tmp.mkpath("adir/dir2/file"))
                ),
            ]
        );
    }

    #[test]
    fn two_watchers_one_file() {
        let (tx, rx) = unbounded();
        let tmp = tempdir::TempDir::new("xi-test-two-watchers").unwrap();
        tmp.create("my_file");
        sleep_if_macos(30_100);
        let mut w = FileWatcher::new(tx);
        w.watch(&tmp.mkpath("my_file"), false, 1.into());
        sleep_if_macos(10);
        w.watch(&tmp.mkpath("my_file"), false, 2.into());
        sleep_if_macos(10);
        tmp.write("my_file");

        let _ = recv_all(&rx, Duration::from_millis(1000));
        let events = w.take_events();
        assert_eq!(
            events,
            vec![
                (
                    1.into(),
                    Event::new(EventKind::Modify(ModifyKind::Any))
                        .add_path(tmp.mkpath("my_file"))
                        .set_flag(Flag::Notice)
                ),
                (
                    2.into(),
                    Event::new(EventKind::Modify(ModifyKind::Any))
                        .add_path(tmp.mkpath("my_file"))
                        .set_flag(Flag::Notice)
                ),
                (
                    1.into(),
                    Event::new(EventKind::Modify(ModifyKind::Any)).add_path(tmp.mkpath("my_file"))
                ),
                (
                    2.into(),
                    Event::new(EventKind::Modify(ModifyKind::Any)).add_path(tmp.mkpath("my_file"))
                ),
            ]
        );

        assert_eq!(w.state.lock().unwrap().watchees.len(), 2);
        w.unwatch(&tmp.mkpath("my_file"), 1.into());
        assert_eq!(w.state.lock().unwrap().watchees.len(), 1);
        sleep_if_macos(1000);
        let path = tmp.mkpath("my_file");
        tmp.remove("my_file");
        sleep_if_macos(1000);
        let _ = recv_all(&rx, Duration::from_millis(1000));
        let events = w.take_events();
        assert!(events.contains(&(
            2.into(),
            Event::new(EventKind::Remove(RemoveKind::Any))
                .add_path(path.clone())
                .set_flag(Flag::Notice)
        )));
        assert!(!events.contains(&(
            1.into(),
            Event::new(EventKind::Remove(RemoveKind::Any)).add_path(path).set_flag(Flag::Notice)
        )));
    }
}
