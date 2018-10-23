// Copyright 2016 The xi-editor Authors.
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

extern crate xi_core_lib as xi_core;
extern crate xi_plugin_lib;
extern crate xi_rope;

use std::path::Path;

use xi_core::ConfigTable;
use xi_plugin_lib::{mainloop, ChunkCache, Error, Plugin, View};
use xi_rope::delta::Builder as EditBuilder;
use xi_rope::interval::Interval;
use xi_rope::rope::RopeDelta;

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

    fn config_changed(&mut self, _view: &mut View<Self::Cache>, _changes: &ConfigTable) {}

    fn update(
        &mut self,
        view: &mut View<Self::Cache>,
        delta: Option<&RopeDelta>,
        _edit_type: String,
        _author: String,
    ) {
        //NOTE: example simple conditional edit. If this delta is
        //an insert of a single '!', we capitalize the preceding word.
        if let Some(delta) = delta {
            let (iv, _) = delta.summary();
            let text: String = delta.as_simple_insert().map(String::from).unwrap_or_default();
            if text == "!" {
                let _ = self.capitalize_word(view, iv.end());
            }
        }
    }
}

impl SamplePlugin {
    /// Uppercases the word preceding `end_offset`.
    fn capitalize_word(&self, view: &mut View<ChunkCache>, end_offset: usize) -> Result<(), Error> {
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

        let new_text = view.get_line(line_nb)?[word_start..end_offset - line_start].to_uppercase();
        let buf_size = view.get_buf_size();
        let mut builder = EditBuilder::new(buf_size);
        let iv = Interval::new_closed_open(line_start + word_start, end_offset);
        builder.replace(iv, new_text.into());
        view.edit(builder.build(), 0, false, true, "sample".into());
        Ok(())
    }
}

fn main() {
    let mut plugin = SamplePlugin;
    mainloop(&mut plugin).unwrap();
}
