// Copyright 2018 Google Inc. All rights reserved.
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
use std::io::{self, Read, Write};
use std::fmt;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::str;
use std::time::SystemTime;

use xi_rpc::RemoteError;
use xi_rope::Rope;

use tabs::BufferId;

#[cfg(feature = "notify")]
use tabs::OPEN_FILE_EVENT_TOKEN;
#[cfg(feature = "notify")]
use watcher::FileWatcher;

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
}

pub enum FileError {
    Io(io::Error, PathBuf),
    UnknownEncoding(PathBuf),
    HasChanged(PathBuf),
}

#[derive(Debug, Clone, Copy)]
pub enum CharacterEncoding {
    Utf8,
    Utf8WithBom
}

impl FileManager {
    #[cfg(feature = "notify")]
    pub fn new(watcher: FileWatcher) -> Self {
        FileManager {
            open_files: HashMap::new(),
            file_info: HashMap::new(),
            watcher,
        }
    }

    #[cfg(not(feature = "notify"))]
    pub fn new() -> Self {
        FileManager {
            open_files: HashMap::new(),
            file_info: HashMap::new(),
        }
    }

    #[cfg(feature = "notify")]
    pub fn watcher(&mut self) -> &mut FileWatcher {
        &mut self.watcher
    }

    pub fn get_info(&self, id: BufferId) -> Option<&FileInfo> {
        self.file_info.get(&id)
    }

    pub fn get_editor(&self, path: &Path) -> Option<BufferId> {
        self.open_files.get(path).map(|id| *id)
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

    pub fn open(&mut self, path: &Path, id: BufferId)
        -> Result<Rope, FileError>
    {
        if !path.exists() {
            let _ = File::create(path).map_err(|e| FileError::Io(e, path.to_owned()))?;
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
        }
    }

    pub fn save(&mut self, path: &Path, text: &Rope, id: BufferId)
        -> Result<(), FileError>
    {
        let is_existing = self.file_info.contains_key(&id);
        if is_existing {
            self.save_existing(path, text, id)
        } else {
            self.save_new(path, text, id)
        }
    }

    fn save_new(&mut self, path: &Path, text: &Rope, id: BufferId)
        -> Result<(), FileError>
    {
        try_save(path, text, CharacterEncoding::Utf8).map_err(|e| FileError::Io(e, path.to_owned()))?;
        let info = FileInfo {
            encoding: CharacterEncoding::Utf8,
            path: path.to_owned(),
            mod_time: get_mod_time(path),
            has_changed: false,
        };
        self.open_files.insert(path.to_owned(), id);
        self.file_info.insert(id, info);
        #[cfg(feature = "notify")]
        self.watcher.watch(path, false, OPEN_FILE_EVENT_TOKEN);
        Ok(())
    }

    fn save_existing(&mut self, path: &Path, text: &Rope, id: BufferId)
        -> Result<(), FileError>
    {
        let prev_path = self.file_info.get(&id).unwrap().path.clone();
        if prev_path != path {
            self.save_new(path, text, id)?;
            self.open_files.remove(&prev_path);
            #[cfg(feature = "notify")]
            self.watcher.unwatch(&prev_path, OPEN_FILE_EVENT_TOKEN);
        } else if self.file_info.get(&id).unwrap().has_changed {
            return Err(FileError::HasChanged(path.to_owned()));
        } else {
            let encoding = self.file_info.get(&id).unwrap().encoding;
            try_save(path, text, encoding).map_err(|e| FileError::Io(e, path.to_owned()))?;
            self.file_info.get_mut(&id).unwrap()
                .mod_time = get_mod_time(path);
        }
        Ok(())
    }
}

fn try_load_file<P>(path: P) -> Result<(Rope, FileInfo), FileError>
where P: AsRef<Path>
{
    // TODO: support for non-utf8
    // it's arguable that the rope crate should have file loading functionality
    let mut f = File::open(path.as_ref()).map_err(|e| FileError::Io(e, path.as_ref().to_owned()))?;
    let mod_time = f.metadata().map_err(|e| FileError::Io(e, path.as_ref().to_owned()))?.modified().ok();
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).map_err(|e| FileError::Io(e, path.as_ref().to_owned()))?;

    let encoding = CharacterEncoding::guess(&bytes);
    let rope = try_decode(bytes, encoding, path.as_ref())?;
    let info = FileInfo {
        encoding,
        mod_time,
        path: path.as_ref().to_owned(),
        has_changed: false,
    };
    Ok((rope, info))
}

fn try_save(path: &Path, text: &Rope, encoding: CharacterEncoding)
    -> io::Result<()>
{
    let mut f = File::create(path)?;
        match encoding {
            CharacterEncoding::Utf8WithBom => f.write_all(UTF8_BOM.as_bytes())?,
            CharacterEncoding::Utf8 => (),
        }

        for chunk in text.iter_chunks(0, text.len()) {
            f.write_all(chunk.as_bytes())?;
        }
        Ok(())
}

fn try_decode(bytes: Vec<u8>,
              encoding: CharacterEncoding, path: &Path) -> Result<Rope, FileError> {
    match encoding {
        CharacterEncoding::Utf8 =>
            Ok(Rope::from(str::from_utf8(&bytes).map_err(|_e| FileError::UnknownEncoding(path.to_owned()))?)),
        CharacterEncoding::Utf8WithBom => {
            let s = String::from_utf8(bytes).map_err(|_e| FileError::UnknownEncoding(path.to_owned()))?;
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
where P: AsRef<Path>
{
    File::open(path)
        .and_then(|f| f.metadata())
        .and_then(|meta| meta.modified())
        .ok()
}

impl From<FileError> for RemoteError {
    fn from(src: FileError) -> RemoteError {
        //TODO: when we migrate to using the failure crate for error handling,
        // this should return a better message
		use std::error::Error;
		let mut code = 0;
		let mut message = String::new();
		match src {
			FileError::Io(e, p) => {
				code = 5;
				match p.to_str() {
					Some(s) => {
						message.push_str(e.description());
						message.push_str(" ");
						message.push_str(s);
					},
					None => {
						message.push_str(e.description());
					}
				};
			},
			FileError::UnknownEncoding(p) => {
				code = 6;
				match p.to_str() {
					Some(s) => {
						message.push_str("Error decoding file: ");
						message.push_str(s);
					},
					None => {
						message.push_str("Error decoding file");
					}
				};
			},
			FileError::HasChanged(p) => {
				code = 7;
				match p.to_str() {
					Some(s) => {
						message.push_str("File has changed on disk: ");
						message.push_str(s);
					},
					None => {
						message.push_str("File has changed on disk");
					}
				};
			}
		};
		RemoteError::custom(code, message, None)
    }
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &FileError::Io(ref e, ref _p) => write!(f, "{}", e),
            &FileError::UnknownEncoding(ref _p) => write!(f, "Error decoding file"),
            &FileError::HasChanged(ref _p) => write!(f, "File has changed on disk. \
            Please save elsewhere and reload the file."),
        }
    }
}

