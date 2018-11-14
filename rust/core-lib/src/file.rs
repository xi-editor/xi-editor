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

const UTF8_BOM: &str = "\u{feff}";

/// Tracks all state related to open files.
pub struct FileManager {
    loading_files: HashMap<PathBuf, BufferId>,
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
    pub loaded_state: FileLoadState,
}
impl FileInfo {
    pub fn try_clone(&self) -> io::Result<FileInfo> {
        let cloned_loaded_state = self.loaded_state.try_clone()?;

        let cloned_file_info = FileInfo {
            encoding: self.encoding.clone(),
            path: self.path.clone(),
            mod_time: self.mod_time.clone(),
            has_changed: self.has_changed.clone(),
            loaded_state: cloned_loaded_state,
        };

        Ok(cloned_file_info)
    }
}

pub enum FileError {
    Io(io::Error, PathBuf),
    UnknownEncoding(PathBuf),
    HasChanged(PathBuf),
    StillLoading(PathBuf),
}

#[derive(Debug, Clone, Copy)]
pub enum CharacterEncoding {
    Utf8,
    Utf8WithBom,
}

#[derive(Debug)]
pub enum FileLoadState {
    FullyLoaded,
    Loading {
        file_handle: File, // This also stores the current seek cursor for the file
        leftovers: Vec<u8>,
    },
}
impl FileLoadState {
    pub fn try_clone(&self) -> io::Result<FileLoadState> {
        let cloned_load_state = match self {
            FileLoadState::FullyLoaded => FileLoadState::FullyLoaded,
            FileLoadState::Loading { file_handle, leftovers } => {
                let cloned_file_handle = file_handle.try_clone()?;

                FileLoadState::Loading {
                    file_handle: cloned_file_handle,
                    leftovers: leftovers.clone(),
                }
            }
        };

        Ok(cloned_load_state)
    }
}

impl FileManager {
    #[cfg(feature = "notify")]
    pub fn new(watcher: FileWatcher) -> Self {
        FileManager {
            loading_files: HashMap::new(),
            open_files: HashMap::new(),
            file_info: HashMap::new(),
            watcher,
        }
    }

    #[cfg(not(feature = "notify"))]
    pub fn new() -> Self {
        FileManager {
            loading_files: HashMap::new(),
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

        let (rope, info) = try_load_file(path)?;

        match info.loaded_state {
            FileLoadState::FullyLoaded => self.open_files.insert(path.to_owned(), id),
            FileLoadState::Loading { .. } => self.loading_files.insert(path.to_owned(), id),
        };

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
        let info = FileInfo {
            encoding: CharacterEncoding::Utf8,
            path: path.to_owned(),
            mod_time: get_mod_time(path),
            has_changed: false,
            loaded_state: FileLoadState::FullyLoaded,
        };
        self.open_files.insert(path.to_owned(), id);
        self.file_info.insert(id, info);
        #[cfg(feature = "notify")]
        self.watcher.watch(path, false, OPEN_FILE_EVENT_TOKEN);
        Ok(())
    }

    fn save_existing(&mut self, path: &Path, text: &Rope, id: BufferId) -> Result<(), FileError> {
        let prev_path = self.previous_path(&id);

        if !self.is_file_loaded(&id) {
            return Err(FileError::StillLoading(path.to_owned()));
        } else if prev_path != path {
            self.save_new(path, text, id)?;
            self.open_files.remove(&prev_path);
            #[cfg(feature = "notify")]
            self.watcher.unwatch(&prev_path, OPEN_FILE_EVENT_TOKEN);
        } else if self.file_info[&id].has_changed {
            return Err(FileError::HasChanged(path.to_owned()));
        } else {
            let existing_file_info = self.file_info.get_mut(&id).unwrap();
            let encoding = existing_file_info.encoding;
            try_save(path, text, encoding).map_err(|e| FileError::Io(e, path.to_owned()))?;
            existing_file_info.mod_time = get_mod_time(path);
        }
        Ok(())
    }

    fn previous_path(&self, id: &BufferId) -> PathBuf {
        let existing_file_info = self.file_info.get(id).unwrap();

        existing_file_info.path.clone()
    }

    pub fn pop_file_info(&mut self, id: &BufferId) -> Option<FileInfo> {
        self.file_info.remove(id)
    }
    pub fn is_file_loaded(&self, id: &BufferId) -> bool {
        self.file_info.get(id).iter().any(|file_info| match file_info.loaded_state {
            FileLoadState::FullyLoaded => true,
            _ => false,
        })
    }
    pub fn update_file_load_state(
        &mut self,
        prev_file_info: FileInfo,
        path: &PathBuf,
        id: &BufferId,
        new_load_state: FileLoadState,
    ) {
        match new_load_state {
            FileLoadState::FullyLoaded => {
                // Move this buffer from loading_files to open_files
                self.loading_files.remove(&path.to_owned());
                self.open_files.insert(path.to_owned(), id.clone());
            }
            FileLoadState::Loading { .. } => (),
        };

        let new_file_info = FileInfo { loaded_state: new_load_state, ..prev_file_info };
        self.file_info.insert(id.clone(), new_file_info);
    }

    pub fn loading_files(&self) -> HashMap<PathBuf, BufferId> {
        self.loading_files.clone()
    }
}

const CHUNK_SIZE: usize = 4096;

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

    let (rope, loaded_state, encoding) = try_load_file_chunk(&mut f, Vec::new(), path.as_ref())?;

    let info = FileInfo {
        encoding,
        mod_time,
        path: path.as_ref().to_owned(),
        has_changed: false,
        loaded_state,
    };
    Ok((rope, info))
}

pub fn try_load_file_chunk<P>(
    file_handle: &mut File,
    mut chunk: Vec<u8>,
    path: P,
) -> Result<(Rope, FileLoadState, CharacterEncoding), FileError>
where
    P: AsRef<Path>,
{
    let mut bytes = [0; CHUNK_SIZE];
    let num_bytes_read =
        file_handle.read(&mut bytes).map_err(|e| FileError::Io(e, path.as_ref().to_owned()))?;

    chunk.extend_from_slice(&bytes);

    let encoding = CharacterEncoding::guess(&chunk);
    let (rope, new_leftovers) = try_decode(&chunk, encoding, path.as_ref())?;

    let new_loaded_state = match (num_bytes_read < CHUNK_SIZE, new_leftovers.is_empty()) {
        (true, true) => FileLoadState::FullyLoaded,
        (true, false) => Err(FileError::UnknownEncoding(path.as_ref().to_owned()))?,
        // Maybe more bytes to read
        _ => {
            let file_handle_copy =
                file_handle.try_clone().map_err(|e| FileError::Io(e, path.as_ref().to_owned()))?;

            FileLoadState::Loading { file_handle: file_handle_copy, leftovers: new_leftovers }
        }
    };

    Ok((rope, new_loaded_state, encoding))
}

fn try_save(path: &Path, text: &Rope, encoding: CharacterEncoding) -> io::Result<()> {
    let tmp_extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map_or_else(|| "swp".to_string(), |extension| format!("{}.swp", extension));
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

fn try_decode(
    bytes: &[u8],
    encoding: CharacterEncoding,
    path: &Path,
) -> Result<(Rope, Vec<u8>), FileError> {
    // Check the last valid UTF-8 character in the chunk
    // If it's incomplete but otherwise valid, save it for the next chunk
    let (complete_bytes, leftovers): (&[u8], &[u8]) = match last_utf8_char_info(bytes) {
        LastUTF8CharInfo::Complete => (bytes, &[]),
        LastUTF8CharInfo::Incomplete(reverse_idx) => bytes.split_at(bytes.len() - reverse_idx),
        LastUTF8CharInfo::InvalidUTF8 => Err(FileError::UnknownEncoding(path.to_owned()))?,
    };

    match encoding {
        CharacterEncoding::Utf8 => {
            let utf8_str = str::from_utf8(complete_bytes)
                .map_err(|_e| FileError::UnknownEncoding(path.to_owned()))?;
            Ok((Rope::from(utf8_str), leftovers.to_vec()))
        }
        CharacterEncoding::Utf8WithBom => {
            let utf8_str = String::from_utf8(complete_bytes.to_vec())
                .map_err(|_e| FileError::UnknownEncoding(path.to_owned()))?;
            Ok((Rope::from(&utf8_str[UTF8_BOM.len()..]), leftovers.to_vec()))
        }
    }
}

enum LastUTF8CharInfo {
    Complete,
    Incomplete(usize),
    InvalidUTF8,
}

fn last_utf8_char_info(s: &[u8]) -> LastUTF8CharInfo {
    // Using the Err here to short circuit the fold, Ok to continue backwards along the slice for continuation bytes
    let last_char_result = s.iter().try_rfold(1, |reverse_idx, byte| match *byte {
        _ if reverse_idx > 4 => Err(LastUTF8CharInfo::InvalidUTF8),
        // First bit is 0, a single byte UTF-8 character
        leading_byte_single if leading_byte_single < 128 => Err(match reverse_idx {
            1 => LastUTF8CharInfo::Complete,
            _ => LastUTF8CharInfo::InvalidUTF8,
        }),
        // Byte starts with 10, a UTF-8 continuation byte
        // This is bit magic equivalent to: 128 <= b || b < 192
        continuation_byte if (continuation_byte as i8) < -0x40 => Ok(reverse_idx + 1),
        // Byte starts with 110, a 2 byte UTF-8 character
        leading_byte_double if 192 <= leading_byte_double && leading_byte_double < 224 => {
            Err(match reverse_idx {
                1 => LastUTF8CharInfo::Incomplete(reverse_idx),
                2 => LastUTF8CharInfo::Complete,
                _ => LastUTF8CharInfo::InvalidUTF8,
            })
        }
        // Byte starts with 1110, a 3 byte UTF-8 character
        leading_byte_triple if 224 <= leading_byte_triple && leading_byte_triple < 240 => {
            Err(match reverse_idx {
                1 | 2 => LastUTF8CharInfo::Incomplete(reverse_idx),
                3 => LastUTF8CharInfo::Complete,
                _ => LastUTF8CharInfo::InvalidUTF8,
            })
        }
        // Byte starts with 11110, a 4 byte UTF-8 character
        leading_byte_quad if 240 <= leading_byte_quad && leading_byte_quad < 248 => {
            Err(match reverse_idx {
                1 | 2 | 3 => LastUTF8CharInfo::Incomplete(reverse_idx),
                4 => LastUTF8CharInfo::Complete,
                _ => LastUTF8CharInfo::InvalidUTF8,
            })
        }
        _ => Err(LastUTF8CharInfo::InvalidUTF8), // Should only happen in invalid UTF-8
    });

    match last_char_result {
        Ok(_) => LastUTF8CharInfo::InvalidUTF8, // Found too many continuation characters!
        Err(result) => result,
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
            &FileError::Io(_, _) => 5,
            &FileError::UnknownEncoding(_) => 6,
            &FileError::HasChanged(_) => 7,
            &FileError::StillLoading(_) => 8,
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
            &FileError::StillLoading(ref p) => {
                write!(f, "File is in the middle of loading, path {:?}", p)
            }
        }
    }
}
