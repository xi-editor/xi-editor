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
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::str;
use std::time::SystemTime;
#[cfg(target_family = "unix")]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};

use xi_rope::Rope;
use xi_rpc::RemoteError;

use crate::tabs::BufferId;
#[cfg(feature = "notify")]
use crate::tabs::{OPEN_FILE_EVENT_TOKEN, OPEN_FILE_TAIL_EVENT_TOKEN};
#[cfg(feature = "notify")]
use crate::watcher::FileWatcher;

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
    #[cfg(target_family = "unix")]
    pub permissions: Option<u32>,
    #[cfg(feature = "notify")]
    pub tail_position: Option<u64>,
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

    pub fn tail(&mut self, path: &Path, id: BufferId) -> Result<Rope, FileError> {
        if !path.exists() {
            return Ok(Rope::from(""));
        }

        let existing_file_info = self.file_info.get(&id).expect("File info must be present");
        let (rope, info) = try_tailing_file(path, existing_file_info)?;
        self.file_info.insert(id, info);
        Ok(rope)
    }

    pub fn open(&mut self, path: &Path, id: BufferId) -> Result<Rope, FileError> {
        if !path.exists() {
            return Ok(Rope::from(""));
        }

        let (rope, info) = try_load_file(path)?;

        self.open_files.insert(path.to_owned(), id);
        if self.file_info.insert(id, info).is_none() {
            #[cfg(feature = "notify")]
            self.watcher.watch(path, false, OPEN_FILE_EVENT_TOKEN);
        }
        Ok(rope)
    }

    pub fn close(&mut self, id: BufferId) {
        if let Some(info) = self.file_info.remove(&id) {
            self.open_files.remove(&info.path);
            #[cfg(feature = "notify")]
            self.watcher.unwatch(&info.path, OPEN_FILE_EVENT_TOKEN);
            #[cfg(feature = "notify")]
            self.watcher.unwatch(&info.path, OPEN_FILE_TAIL_EVENT_TOKEN);
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
        try_save(path, text, CharacterEncoding::Utf8, self.get_info(id))
            .map_err(|e| FileError::Io(e, path.to_owned()))?;
        let info = FileInfo {
            encoding: CharacterEncoding::Utf8,
            path: path.to_owned(),
            mod_time: get_mod_time(path),
            has_changed: false,
            #[cfg(target_family = "unix")]
            permissions: get_permissions(path),
            #[cfg(feature = "notify")]
            tail_position: None,
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
            #[cfg(feature = "notify")]
            self.watcher.unwatch(&prev_path, OPEN_FILE_TAIL_EVENT_TOKEN);
        } else if self.file_info[&id].has_changed {
            return Err(FileError::HasChanged(path.to_owned()));
        } else {
            let encoding = self.file_info[&id].encoding;
            try_save(path, text, encoding, self.get_info(id))
                .map_err(|e| FileError::Io(e, path.to_owned()))?;
            self.file_info.get_mut(&id).unwrap().mod_time = get_mod_time(path);
        }
        Ok(())
    }

    #[cfg(feature = "notify")]
    pub fn update_current_position_in_tail(
        &mut self,
        buffer_id: BufferId,
    ) -> Result<bool, FileError> {
        if let Some(file_info) = self.file_info.get_mut(&buffer_id) {
            let path = &file_info.path.to_owned();

            let mut f = File::open(path).map_err(|e| FileError::Io(e, path.to_owned()))?;
            let end_position =
                f.seek(SeekFrom::End(0)).map_err(|e| FileError::Io(e, path.to_owned()))?;

            file_info.tail_position = Some(end_position);
            self.watcher.unwatch(path, OPEN_FILE_EVENT_TOKEN);
            self.watcher.watch(path, false, OPEN_FILE_TAIL_EVENT_TOKEN);
            Ok(true)
        } else {
            info!("Non-existent file cannot be tailed!");
            Ok(false)
        }
    }

    #[cfg(feature = "notify")]
    pub fn disable_tailing(&mut self, buffer_id: BufferId) -> bool {
        if let Some(file_info) = self.file_info.get_mut(&buffer_id) {
            let path = &file_info.path.to_owned();
            file_info.tail_position = None;
            self.watcher.unwatch(path, OPEN_FILE_TAIL_EVENT_TOKEN);
            self.watcher.watch(path, false, OPEN_FILE_EVENT_TOKEN);
            true
        } else {
            info!("Non-existent file cannot be tailed!");
            false
        }
    }
}

/// When tailing a file, instead of reading file from beginning, we need to get changes from the end.
/// This method does that.
#[cfg(feature = "notify")]
fn try_tailing_file<P>(path: P, file_info: &FileInfo) -> Result<(Rope, FileInfo), FileError>
where
    P: AsRef<Path>,
{
    let mut f =
        File::open(path.as_ref()).map_err(|e| FileError::Io(e, path.as_ref().to_owned()))?;
    let end_position =
        f.seek(SeekFrom::End(0)).map_err(|e| FileError::Io(e, path.as_ref().to_owned()))?;

    let current_position = file_info.tail_position.expect(
        "tail_position is None. Since we are tailing a file, tail_position cannot be None.",
    );
    let diff = end_position - current_position;
    let mut buf = vec![0; diff as usize];
    f.seek(SeekFrom::Current(-(buf.len() as i64))).unwrap();
    f.read_exact(&mut buf).unwrap();

    let mod_time =
        f.metadata().map_err(|e| FileError::Io(e, path.as_ref().to_owned()))?.modified().ok();

    let encoding = file_info.encoding;
    let permissions = file_info.permissions;
    let rope = try_decode(buf, encoding, path.as_ref())?;
    let info = FileInfo {
        encoding,
        mod_time,
        path: path.as_ref().to_owned(),
        has_changed: false,
        permissions,
        tail_position: Some(end_position),
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
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).map_err(|e| FileError::Io(e, path.as_ref().to_owned()))?;

    let encoding = CharacterEncoding::guess(&bytes);
    let rope = try_decode(bytes, encoding, path.as_ref())?;
    let info = FileInfo {
        encoding,
        mod_time: get_mod_time(&path),
        #[cfg(target_family = "unix")]
        permissions: get_permissions(&path),
        path: path.as_ref().to_owned(),
        has_changed: false,
        #[cfg(feature = "notify")]
        tail_position: None,
    };
    Ok((rope, info))
}

#[allow(unused)]
fn try_save(
    path: &Path,
    text: &Rope,
    encoding: CharacterEncoding,
    file_info: Option<&FileInfo>,
) -> io::Result<()> {
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

    #[cfg(target_family = "unix")]
    {
        if let Some(info) = file_info {
            fs::set_permissions(path, Permissions::from_mode(info.permissions.unwrap_or(0o644)))
                .unwrap_or_else(|e| {
                    warn!("Couldn't set permissions on file {} due to error {}", path.display(), e)
                });
        }
    }

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
fn get_mod_time<P: AsRef<Path>>(path: P) -> Option<SystemTime> {
    File::open(path).and_then(|f| f.metadata()).and_then(|meta| meta.modified()).ok()
}

/// Returns the file permissions for the file at a given path on UNIXy systems,
/// if present.
#[cfg(target_family = "unix")]
fn get_permissions<P: AsRef<Path>>(path: P) -> Option<u32> {
    File::open(path).and_then(|f| f.metadata()).map(|meta| meta.permissions().mode()).ok()
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
