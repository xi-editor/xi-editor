// Copyright 2018 The xi-editor Authors.
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

//! Interactions with the file system.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::str;
use std::time::SystemTime;

use xi_rope::Rope;
use xi_rpc::RemoteError;

use tabs::BufferId;

#[cfg(feature = "notify")]
use tabs::OPEN_FILE_EVENT_TOKEN;
#[cfg(feature = "notify")]
use watcher::FileWatcher;
use std::io::Seek;
use std::io::SeekFrom;
use xi_rope::tree::Node;

//sj_todo why was this imported to begin with?
//use file::FileInfo;

const UTF8_BOM: &str = "\u{feff}";

/// Tracks all state related to open files.
pub struct FileManager {
    open_files: HashMap<PathBuf, BufferId>,
    file_info: HashMap<BufferId, FileInfo>,
    /// A monitor of filesystem events, for things like reloading changed files.
    #[cfg(feature = "notify")]
    watcher: FileWatcher,
}

#[derive(Debug)]
pub struct FileInfo {
    pub encoding: CharacterEncoding,
    pub path: PathBuf,
    pub mod_time: Option<SystemTime>,
    pub has_changed: bool,
    #[cfg(feature = "notify")]
    pub tail_details: TailDetails,
}

#[derive(Debug)]
pub struct TailDetails {
    pub current_position_in_tail: u64,
    pub is_tail_mode_on: bool,
    pub is_at_bottom_of_file: bool,
}

pub enum FileError {
    Io(io::Error, PathBuf),
    UnknownEncoding(PathBuf),
    HasChanged(PathBuf),
}

#[derive(Debug, Clone, Copy)]
pub enum CharacterEncoding {
    Utf8,
    Utf8WithBom,
}

impl FileManager {
    #[cfg(feature = "notify")]
    pub fn new(watcher: FileWatcher) -> Self {
        FileManager { open_files: HashMap::new(), file_info: HashMap::new(), watcher }
    }

    #[cfg(not(feature = "notify"))]
    pub fn new() -> Self {
        FileManager { open_files: HashMap::new(), file_info: HashMap::new() }
    }

    #[cfg(feature = "notify")]
    pub fn watcher(&mut self) -> &mut FileWatcher {
        &mut self.watcher
    }

    pub fn get_info(&self, id: BufferId) -> Option<&FileInfo> {
        self.file_info.get(&id)
    }

    pub fn get_current_position_in_tail(&self, id: &BufferId) -> u64 {
        match self.file_info.get(id) {
            Some(V) => V.tail_details.current_position_in_tail,
            None => 0
        }
    }

    pub fn get_editor(&self, path: &Path) -> Option<BufferId> {
        self.open_files.get(path).cloned()
    }

    /// Returns `true` if this file is open and has changed on disk.
    /// This state is stashed.
    pub fn check_file(&mut self, path: &Path, id: BufferId) -> bool {
        if let Some(info) = self.file_info.get_mut(&id) {
            let mod_t = get_mod_time(path);
            if mod_t != info.mod_time {
                info.has_changed = true
            }
            return info.has_changed;
        }
        false
    }

    pub fn open(&mut self, path: &Path, id: BufferId) -> Result<Rope, FileError> {
        if !path.exists() {
            let _ = File::create(path).map_err(|e| FileError::Io(e, path.to_owned()))?;
        }

        if cfg!(feature = "notify") {

            let current_position = self.get_current_position_in_tail(&id);
            info!("current_position {:?}", current_position);
            let (rope, info) = try_tailing_file(path, current_position)?;

            self.open_files.insert(path.to_owned(), id);
            if self.file_info.insert(id, info).is_none() {
                self.watcher.watch(path, false, OPEN_FILE_EVENT_TOKEN);
            }
            Ok(rope)
        } else {
            let (rope, info) = try_load_file(path)?;

            self.open_files.insert(path.to_owned(), id);
            self.file_info.insert(id, info);
            Ok(rope)
        }
    }

    pub fn close(&mut self, id: BufferId) {
        if let Some(info) = self.file_info.remove(&id) {
            self.open_files.remove(&info.path);
            #[cfg(feature = "notify")]
            self.watcher.unwatch(&info.path, OPEN_FILE_EVENT_TOKEN);
        }
    }

    pub fn save(&mut self, path: &Path, text: &Rope, id: BufferId) -> Result<(), FileError> {
        let is_existing = self.file_info.contains_key(&id);
        if is_existing {
            self.save_existing(path, text, id)
        } else {
            self.save_new(path, text, id)
        }
    }

    fn save_new(&mut self, path: &Path, text: &Rope, id: BufferId) -> Result<(), FileError> {
        try_save(path, text, CharacterEncoding::Utf8)
            .map_err(|e| FileError::Io(e, path.to_owned()))?;

        #[cfg(feature = "notify")]
        let new_tail_details = TailDetails {
            current_position_in_tail: 0,
            is_tail_mode_on: false,
            is_at_bottom_of_file: false,
        };

        let info = FileInfo {
            encoding: CharacterEncoding::Utf8,
            path: path.to_owned(),
            mod_time: get_mod_time(path),
            has_changed: false,
            #[cfg(feature = "notify")]
            tail_details: new_tail_details,
        };

        self.open_files.insert(path.to_owned(), id);
        self.file_info.insert(id, info);
        #[cfg(feature = "notify")]
        self.watcher.watch(path, false, OPEN_FILE_EVENT_TOKEN);
        Ok(())
    }

    fn save_existing(&mut self, path: &Path, text: &Rope, id: BufferId) -> Result<(), FileError> {
        let prev_path = self.file_info[&id].path.clone();
        if prev_path != path {
            self.save_new(path, text, id)?;
            self.open_files.remove(&prev_path);
            #[cfg(feature = "notify")]
            self.watcher.unwatch(&prev_path, OPEN_FILE_EVENT_TOKEN);
        } else if self.file_info[&id].has_changed {
            return Err(FileError::HasChanged(path.to_owned()));
        } else {
            let encoding = self.file_info[&id].encoding;
            #[cfg(feature = "notify")]
            self.watcher.unwatch(&path, OPEN_FILE_EVENT_TOKEN);
            try_save(path, text, encoding).map_err(|e| FileError::Io(e, path.to_owned()))?;
            self.file_info.get_mut(&id).unwrap().mod_time = get_mod_time(path);
            #[cfg(feature = "notify")]
            self.watcher.watch(&path,false,OPEN_FILE_EVENT_TOKEN);

            #[cfg(feature = "notify")]
            self.update_current_position_in_tail(path, &id);
        }
        Ok(())
    }

    pub fn update_current_position_in_tail(&mut self, path: &Path, id: &BufferId) -> Result<(), FileError> {

        let existing_file_info = self.file_info.get_mut(&id).unwrap();

        let mut f = File::open(path).map_err(|e| FileError::Io(e, path.to_owned()))?;
        let end_position = f.seek(SeekFrom::End(0)).map_err(|e| FileError::Io(e, path.to_owned()))?;

        existing_file_info.tail_details.current_position_in_tail = end_position;
        info!("Saving with end_position {:?}", end_position);
        Ok(())
    }
}

/// When tailing a file, instead of reading file from beginning, we need to get changes from the end.
/// This method does that.
fn try_tailing_file<P>(path: P, current_position: u64) -> Result<(Rope, FileInfo), FileError>
    where P: AsRef<Path>
{
    let mut f = File::open(path.as_ref()).map_err(|e| FileError::Io(e, path.as_ref().to_owned()))?;
    let end_position = f.seek(SeekFrom::End(0)).map_err(|e| FileError::Io(e, path.as_ref().to_owned()))?;

    let diff = end_position - current_position;
    info!("end_position {:?} current_position {:?}", end_position, current_position);
    let mut buf = vec![0; diff as usize];
    f.seek(SeekFrom::Current(-(buf.len() as i64))).unwrap();
    f.read_exact(&mut buf).unwrap();

    let new_current_position = end_position;

    let new_tail_details = TailDetails {
        current_position_in_tail: new_current_position,
        is_tail_mode_on: true,
        is_at_bottom_of_file: true,
    };

    let mod_time = f.metadata().map_err(|e| FileError::Io(e, path.as_ref().to_owned()))?.modified().ok();

    let encoding = CharacterEncoding::guess(&buf);
    let rope = try_decode(buf, encoding, path.as_ref())?;
    let info = FileInfo {
        encoding,
        mod_time,
        path: path.as_ref().to_owned(),
        has_changed: false,
        tail_details: new_tail_details,
    };
    Ok((rope, info))
}

fn try_load_file<P>(path: P) -> Result<(Rope, FileInfo), FileError>
where
    P: AsRef<Path>,
{
    // TODO: support for non-utf8
    // it's arguable that the rope crate should have file loading functionality
    let mut f =
        File::open(path.as_ref()).map_err(|e| FileError::Io(e, path.as_ref().to_owned()))?;
    let mod_time =
        f.metadata().map_err(|e| FileError::Io(e, path.as_ref().to_owned()))?.modified().ok();
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).map_err(|e| FileError::Io(e, path.as_ref().to_owned()))?;

    let encoding = CharacterEncoding::guess(&bytes);
    let rope = try_decode(bytes, encoding, path.as_ref())?;

    #[cfg(feature = "notify")]
    let unchanged_tail_details = TailDetails {
        current_position_in_tail: 0,
        is_tail_mode_on: false,
        is_at_bottom_of_file: false,
    };

    let info = FileInfo {
        encoding, mod_time,
        path: path.as_ref().to_owned(),
        has_changed: false,
        #[cfg(feature = "notify")]
        tail_details: unchanged_tail_details };
    Ok((rope, info))
}

fn try_save(path: &Path, text: &Rope, encoding: CharacterEncoding) -> io::Result<()> {
    let tmp_extension = path.extension().map_or_else(
        || OsString::from("swp"),
        |ext| {
            let mut ext = ext.to_os_string();
            ext.push(".swp");
            ext
        },
    );
    let tmp_path = &path.with_extension(tmp_extension);

    let mut f = File::create(tmp_path)?;
    match encoding {
        CharacterEncoding::Utf8WithBom => f.write_all(UTF8_BOM.as_bytes())?,
        CharacterEncoding::Utf8 => (),
    }

    for chunk in text.iter_chunks(..text.len()) {
        f.write_all(chunk.as_bytes())?;
    }

    fs::rename(tmp_path, path)?;

    Ok(())
}

fn try_decode(bytes: Vec<u8>, encoding: CharacterEncoding, path: &Path) -> Result<Rope, FileError> {
    match encoding {
        CharacterEncoding::Utf8 => Ok(Rope::from(
            str::from_utf8(&bytes).map_err(|_e| FileError::UnknownEncoding(path.to_owned()))?,
        )),
        CharacterEncoding::Utf8WithBom => {
            let s = String::from_utf8(bytes)
                .map_err(|_e| FileError::UnknownEncoding(path.to_owned()))?;
            Ok(Rope::from(&s[UTF8_BOM.len()..]))
        }
    }
}

impl CharacterEncoding {
    fn guess(s: &[u8]) -> Self {
        if s.starts_with(UTF8_BOM.as_bytes()) {
            CharacterEncoding::Utf8WithBom
        } else {
            CharacterEncoding::Utf8
        }
    }
}

/// Returns the modification timestamp for the file at a given path,
/// if present.
fn get_mod_time<P>(path: P) -> Option<SystemTime>
where
    P: AsRef<Path>,
{
    File::open(path).and_then(|f| f.metadata()).and_then(|meta| meta.modified()).ok()
}

impl From<FileError> for RemoteError {
    fn from(src: FileError) -> RemoteError {
        //TODO: when we migrate to using the failure crate for error handling,
        // this should return a better message
        let code = src.error_code();
        let message = src.to_string();
        RemoteError::custom(code, message, None)
    }
}

impl FileError {
    fn error_code(&self) -> i64 {
        match self {
            FileError::Io(_, _) => 5,
            FileError::UnknownEncoding(_) => 6,
            FileError::HasChanged(_) => 7,
        }
    }
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            FileError::Io(ref e, ref p) => write!(f, "{}. File path: {:?}", e, p),
            FileError::UnknownEncoding(ref p) => write!(f, "Error decoding file: {:?}", p),
            FileError::HasChanged(ref p) => write!(
                f,
                "File has changed on disk. \
                 Please save elsewhere and reload the file. File path: {:?}",
                p
            ),
        }
    }
}
