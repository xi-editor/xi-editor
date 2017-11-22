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

use notify::{Watcher, RecursiveMode, watcher, DebouncedEvent, RecommendedWatcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::thread;
use std::collections::VecDeque;
use std::fmt;

use xi_rpc::RpcPeer;

/// xi_rpc idle Token for watcher related idle scheduling.
pub const WATCH_IDLE_TOKEN: usize = 1002;

/// Wrapper around a `notify::Watcher`. It runs the inner watcher
/// in a separate thread, and communicates with it via an `mpsc::channel`.
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

pub type EventQueue = VecDeque<(WatchToken, DebouncedEvent)>;

pub type PathFilter = Fn(&Path) -> bool + Send + 'static;

impl FileWatcher {
    pub fn new<T: Notify + 'static>(peer: T) -> Self {
        let (tx_event, rx_event) = channel();

        let state = Arc::new(Mutex::new(WatcherState::default()));
        let state_clone = state.clone();

        let inner = watcher(tx_event, Duration::from_millis(100))
            .expect("watcher should spawn");

        thread::spawn(move || {
            while let Ok(event) = rx_event.recv() {
                let mut state = state_clone.lock().unwrap();
                let WatcherState { ref mut events, ref mut watchees } = *state;

                watchees.iter()
                    .filter(|w| w.wants_event(&event))
                    .map(|w| w.token)
                    .for_each(|t| events.push_back((t, clone_event(&event))));

                peer.notify();
            }
        });

        FileWatcher { inner, state }
    }

    /// Begin watching `path`. As `DebouncedEvent`s (documented in the
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
    pub fn watch_filtered<F>(&mut self, path: &Path, recursive: bool,
                             token: WatchToken, filter: F)
        where F: Fn(&Path) -> bool + Send + 'static,
    {
        let filter = Box::new(filter) as Box<PathFilter>;
        self.watch_impl(path, recursive, token, Some(filter));
    }

    fn watch_impl(&mut self, path: &Path, recursive: bool, token: WatchToken,
                  filter: Option<Box<PathFilter>>)
    {
        let path = match path.canonicalize() {
            Ok(ref p) if p.exists() => p.to_owned(),
            _ => return,
        };

        let mut state = self.state.lock().unwrap();

        let w = Watchee { path, recursive, token, filter };
        let mode = mode_from_bool(w.recursive);

        if !state.watchees.iter().any(|w2| w.path == w2.path) {
            if let Err(e) = self.inner.watch(&w.path, mode) {
                eprintln!("watching error {:?}", e);
            }
        }

        state.watchees.push(w);

    }

    /// Removes the provided token/path pair from the watch list.
    /// Does not stop watching this path, if it is associated with
    /// other tokens.
    pub fn unwatch(&mut self, token: WatchToken, path: &Path) {
        let mut state = self.state.lock().unwrap();

        let idx = state.watchees.iter()
            .position(|w| w.token == token && w.path == path);

        if let Some(idx) = idx {
            let removed = state.watchees.remove(idx);
            if !state.watchees.iter().any(|w| w.path == removed.path) {
                if let Err(e) = self.inner.unwatch(&removed.path) {
                    eprintln!("unwatching error {:?}", e);
                }
            }
        }
    }

    /// Empties the event queue, returning any contained events.
    pub fn drain_events(&self) -> Vec<(WatchToken, DebouncedEvent)> {
        let mut state = self.state.lock().unwrap();
        let WatcherState { ref mut events, .. } = *state;
        let v = events.drain(..).collect();
        v
    }
}

impl Watchee {
    fn wants_event(&self, event: &DebouncedEvent) -> bool {
        use self::DebouncedEvent::*;
        match *event {
            NoticeWrite(ref p) | NoticeRemove(ref p) | Create(ref p) |
                Write(ref p) | Chmod(ref p) | Remove(ref p) => {
                    self.applies_to_path(p)
                }
            Rename(ref p1, ref p2) => {
                self.applies_to_path(p1) || self.applies_to_path(p2)
            }
            Rescan => false,
            Error(_, ref opt_p) => {
                opt_p.as_ref().map(|p| self.applies_to_path(p))
                    .unwrap_or(false)
            }
        }
    }

    fn applies_to_path(&self, path: &Path) -> bool {
        let general_case = if path.starts_with(&self.path) {
            (self.recursive || &self.path == path) ||
                path.parent() == Some(&self.path)
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
        self.schedule_idle(WATCH_IDLE_TOKEN);
    }
}

impl fmt::Debug for Watchee {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Watchee path: {:?}, r {}, t {} f {}",
               self.path, self.recursive, self.token.0, self.filter.is_some())
    }
}

fn mode_from_bool(b: bool) -> RecursiveMode {
    if b {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    }
}

// Debounced event does not implement clone
// TODO: remove if https://github.com/passcod/notify/pull/133 is merged
fn clone_event(event: &DebouncedEvent) -> DebouncedEvent {
    use self::DebouncedEvent::*;
    use notify::Error::*;
    match *event {
        NoticeWrite(ref p) => NoticeWrite(p.to_owned()),
        NoticeRemove(ref p) => NoticeRemove(p.to_owned()),
        Create(ref p) => Create(p.to_owned()),
        Write(ref p) => Write(p.to_owned()),
        Chmod(ref p) => Chmod(p.to_owned()),
        Remove(ref p) => Remove(p.to_owned()),
        Rename(ref p1, ref p2) => Rename(p1.to_owned(), p2.to_owned()),
        Rescan => Rescan,
        Error(ref e, ref opt_p) => {
            let error = match *e {
                PathNotFound => PathNotFound,
                WatchNotFound => WatchNotFound,
                Generic(ref s) => Generic(s.to_owned()),
                Io(ref e) => Generic(format!("{:?}", e)),
            };
            Error(error, opt_p.clone())
        }
    }
}

#[cfg(test)]
extern crate tempdir;

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::thread;
    use std::fs;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use std::io::Write;

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

    impl Notify for mpsc::Sender<bool> {
        fn notify(&self) {
            self.send(true).expect("send shouldn't fail")
        }
    }

    // Sleep for `duration` in milliseconds
    pub fn sleep(duration: u64) {
        thread::sleep(Duration::from_millis(duration));
    }

    // Sleep for `duration` in milliseconds if running on OS X
    pub fn sleep_macos(duration: u64) {
        if cfg!(target_os = "macos") {
            thread::sleep(Duration::from_millis(duration));
        }
    }

    pub fn recv_all<T>(rx: &mpsc::Receiver<T>, duration: Duration) -> Vec<T> {
        let start = Instant::now();
        let mut events = Vec::new();

        while start.elapsed() < duration {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(event) => events.push(event),
                Err(mpsc::RecvTimeoutError::Timeout) => (),
                Err(e) => panic!("unexpected channel err: {:?}", e)
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
            let mut path = self.path().canonicalize()
                .expect("failed to canonalize path").to_owned();
            for part in p.split('/').collect::<Vec<_>>() {
                if part != "." {
                    path.push(part);
                }
            }
            path
        }

        fn create(&self, p: &str) {
            let path = self.mkpath(p);
            if path.components().last().unwrap().as_os_str()
                .to_str().unwrap().contains("dir") {
                    fs::create_dir_all(path)
                        .expect("failed to create directory");
                } else {
                    let parent = path.parent()
                        .expect("failed to get parent directory").to_owned();
                    if !parent.exists() {
                        fs::create_dir_all(parent)
                            .expect("failed to create parent directory");
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
            fs::rename(&path_a, &path_b)
                .expect("failed to rename file or directory");
        }

        fn write(&self, p: &str) {
            let path = self.mkpath(p);

            let mut file = fs::OpenOptions::new()
                .write(true)
                .open(path)
                .expect("failed to open file");

            file.write(b"some data")
                .expect("failed to write to file");
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

        w.filter = Some(Box::new(|p| {
            p.extension().and_then(OsStr::to_str) == Some("txt")
        }));
        assert!(w.applies_to_path(&PathBuf::from("/hi/there/dear/friend.txt")));
        assert!(w.applies_to_path(&PathBuf::from("/hi/there/friend.txt")));
        assert!(!w.applies_to_path(&PathBuf::from("/hi/there/")));
        assert!(!w.applies_to_path(&PathBuf::from("/hi/there/friend.exe")));
        assert!(w.applies_to_path(&PathBuf::from("/hi/there/my/old/sweet/pal.txt")));
    }

    //https://github.com/passcod/notify/issues/131
    #[test]
    fn test_crash_repro() {
        let (tx, _rx) = channel();
        let path = PathBuf::from("/usr/local/bin/git");
        let mut w = watcher(tx, Duration::from_secs(1)).unwrap();
        w.watch(&path, RecursiveMode::NonRecursive).unwrap();
        sleep(20);
        w.watch(&path, RecursiveMode::NonRecursive).unwrap();
        w.unwatch(&path).unwrap();
    }

    #[test]
    fn recurse_with_contained() {
        let (tx, rx) = channel();
        let tmp = tempdir::TempDir::new("xi-test").unwrap();
        let mut w = FileWatcher::new(tx);
        tmp.create("adir/dir2/file");
        sleep_macos(35_000);
        w.watch(&tmp.mkpath("adir"), true, 1.into());
        sleep_macos(10);
        w.watch(&tmp.mkpath("adir/dir2/file"), false,  2.into());
        sleep_macos(10);
        w.unwatch(1.into(), &tmp.mkpath("adir"));
        sleep(10);
        tmp.write("adir/dir2/file");
        let _ = recv_all(&rx, Duration::from_millis(1000));
        let events = w.drain_events();
        assert_eq!(events, vec![
                   (2.into(), DebouncedEvent::NoticeWrite(tmp.mkpath("adir/dir2/file"))),
                   (2.into(), DebouncedEvent::Write(tmp.mkpath("adir/dir2/file"))),
        ]);
    }

    #[test]
    fn two_watchers_one_file() {
        let (tx, rx) = channel();
        let tmp = tempdir::TempDir::new("xi-test").unwrap();
        tmp.create("my_file");
        sleep_macos(25_000);
        let mut w = FileWatcher::new(tx);
        w.watch(&tmp.mkpath("my_file"), false, 1.into());
        sleep_macos(10);
        w.watch(&tmp.mkpath("my_file"), false, 2.into());
        sleep_macos(10);
        tmp.remove("my_file");

        let _ = recv_all(&rx, Duration::from_millis(1000));
        let events = w.drain_events();
        assert_eq!(events, vec![
                   (1.into(), DebouncedEvent::NoticeRemove(tmp.mkpath("my_file"))),
                   (2.into(), DebouncedEvent::NoticeRemove(tmp.mkpath("my_file"))),
                   (1.into(), DebouncedEvent::Remove(tmp.mkpath("my_file"))),
                   (2.into(), DebouncedEvent::Remove(tmp.mkpath("my_file"))),
        ]);

        w.unwatch(1.into(), &tmp.mkpath("my_file"));
        sleep_macos(10);
        tmp.create("my_file");

        let _ = recv_all(&rx, Duration::from_millis(1000));
        let events = w.drain_events();
        assert_eq!(events, vec![
                   (2.into(), DebouncedEvent::Create(tmp.mkpath("my_file"))),
        ]);
    }
}
