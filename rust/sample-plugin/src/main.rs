// Copyright 2016 Google Inc. All rights reserved.
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

//! A sample plugin, intended as an illustration and a template for plugin
//! developers.

extern crate xi_plugin_lib;
extern crate xi_core_lib as xi_core;
extern crate xi_rope;
extern crate xi_rpc;

use std::path::{Path, PathBuf};
use std::fs::DirEntry;

use xi_core::ConfigTable;
use xi_core::plugin_rpc::{CompletionResponse, CompletionItem};

use xi_rope::rope::RopeDelta;
use xi_rope::interval::Interval;
use xi_rope::delta::Builder as EditBuilder;
use xi_rpc::RemoteError;

use xi_plugin_lib::{Plugin, ChunkCache, View, mainloop, Error};

/// A type that implements the `Plugin` trait, and interacts with xi-core.
///
/// Currently, this plugin has a single noteworthy behaviour,
/// intended to demonstrate how to edit a document; when the plugin is active,
/// and the user inserts an exclamation mark, the plugin will capitalize the
/// preceding word.
struct SamplePlugin;

//NOTE: implementing the `Plugin` trait is the sole requirement of a plugin.
// For more documentation, see `rust/plugin-lib` in this repo.
impl Plugin for SamplePlugin {
    type Cache = ChunkCache;

    fn new_view(&mut self, view: &mut View<Self::Cache>) {
        eprintln!("new view {}", view.get_id());
    }

    fn did_close(&mut self, view: &View<Self::Cache>) {
        eprintln!("close view {}", view.get_id());
    }

    fn did_save(&mut self, view: &mut View<Self::Cache>, _old: Option<&Path>) {
        eprintln!("saved view {}", view.get_id());
    }

    fn config_changed(&mut self, _view: &mut View<Self::Cache>, _changes: &ConfigTable) {
    }

    fn update(&mut self, view: &mut View<Self::Cache>, delta: Option<&RopeDelta>,
              _edit_type: String, _author: String) {

        //NOTE: example simple conditional edit. If this delta is
        //an insert of a single '!', we capitalize the preceding word.
        if let Some(delta) = delta {
            let (iv, _) = delta.summary();
            let text: String = delta.as_simple_insert()
                .map(String::from)
                .unwrap_or_default();
            if text == "!" {
                let _ = self.capitalize_word(view, iv.end());
            }
        }
    }

    /// Handles a request for autocomplete, by attempting to complete file paths.
    ///
    /// If the word under the cursor resembles a file path, this fn will attempt to
    /// locate that path and find subitems, which it will return as completion suggestions.
    fn completions(&mut self, view: &mut View<Self::Cache>, request_id: usize, pos: usize) {
        let response = self.path_completions(view, pos)
            .map(|items| CompletionResponse {
                is_incomplete: false,
                can_resolve: false,
                items,
            });

        view.completions(request_id, response)
    }
}

impl SamplePlugin {
    /// Uppercases the word preceding `end_offset`.
    fn capitalize_word(&self, view: &mut View<ChunkCache>, end_offset: usize)
        -> Result<(), Error>
    {
        //NOTE: this makes it clear to me that we need a better API for edits
        let line_nb = view.line_of_offset(end_offset)?;
        let line_start = view.offset_of_line(line_nb)?;

        let mut cur_utf8_ix = 0;
        let mut word_start = 0;
        for c in view.get_line(line_nb)?.chars() {
            if c.is_whitespace() {
                word_start = cur_utf8_ix;
            }

            cur_utf8_ix += c.len_utf8();

            if line_start + cur_utf8_ix == end_offset {
                break;
            }
        }

        let new_text = view.get_line(line_nb)?[word_start..end_offset-line_start]
            .to_uppercase();
        let buf_size = view.get_buf_size();
        let mut builder = EditBuilder::new(buf_size);
        let iv = Interval::new_closed_open(line_start + word_start, end_offset);
        builder.replace(iv, new_text.into());
        view.edit(builder.build(), 0, false, true, "sample".into());
        Ok(())
    }

    /// Attempts to find file path completion suggestions.
    fn path_completions(&self, view: &mut View<ChunkCache>,
                        pos: usize) -> Result<Vec<CompletionItem>, RemoteError> {
        let (word_start, word) = self.get_word_at_offset(view, pos);
        if word.contains('/') {
            let path = self.normalize_path(view.get_path(), &word);
            let parent = if path.is_dir() {
                Some(path.as_path())
            } else {
                path.parent()
            };
            let children = parent.map(|p| self.get_contents(p))
                .ok_or_else(|| RemoteError::custom(420, "not a dir", None))?;
            let result = self.make_completions(view, children, &word, word_start);
            Ok(result)
        } else {
            Err(RemoteError::custom(420, "not path-like", None))
        }
    }

    /// Given a word to complete and a list of viable paths to suggest,
    /// constructs `CompletionItem`s.
    fn make_completions(&self, view: &View<ChunkCache>, children: Vec<DirEntry>,
                        word: &str, word_off: usize) -> Vec<CompletionItem> {
        children.iter()
            .map(|child| {
                let label: String = child.file_name().to_string_lossy().into();
                let mut completion = CompletionItem::with_label(&label);
                if let Some(last_path_cmp_offset) = word.rfind('/') {
                    let delta = RopeDelta::simple_edit(
                        Interval::new_open_closed(word_off + last_path_cmp_offset, word_off + word.len()),
                        label.into(),
                        view.get_buf_size());
                    completion.edit = Some(delta);
                }
                completion
            })
        .collect()
    }

    // NOTE: don't do this
    fn get_contents(&self, path: &Path) -> Vec<DirEntry> {
        let contents: Vec<DirEntry> = path.read_dir()
            .ok()
            .map(|d| d.flat_map(|e| e.ok())
            .collect())
            .unwrap_or_default();
        eprintln!("found {} items in {:?}", contents.len(), &path);
        contents
    }

    fn normalize_path(&self, base: Option<&Path>, word: &str) -> PathBuf {
        match word {
            s if s.starts_with('/') => s.into(),
            s if s.starts_with('~') => {
                let home = ::std::env::home_dir().expect("everyone needs a $HOME");
                eprintln!("$HOME: {:?}", &home);
                home.join(s.trim_left_matches(|c| c == '~' || c == '/'))
            }
            s if base.is_some() => base.unwrap().join(s),
            _ => word.into(),
        }
    }

    fn get_word_at_offset(&self, view: &mut View<ChunkCache>,
                          offset: usize) -> (usize, String) {

        let line_nb = view.line_of_offset(offset).unwrap();
        let line_start = view.offset_of_line(line_nb).unwrap();

        let mut cur_utf8_ix = 0;
        let mut word_start = 0;
        for c in view.get_line(line_nb).unwrap().chars() {
            if c.is_whitespace() {
                word_start = cur_utf8_ix;
            }

            cur_utf8_ix += c.len_utf8();

            if line_start + cur_utf8_ix == offset {
                break;
            }
        }

        let word = view.get_line(line_nb)
            .map(|s| s[word_start..offset-line_start].trim().to_string())
            .unwrap();
        eprintln!("using word '{}' at line {} ({}..{})", &word, line_nb,
                  word_start, offset-line_start);
        (word_start + line_start, word)
    }
}

fn main() {
    let mut plugin = SamplePlugin;
    mainloop(&mut plugin).unwrap();
}
