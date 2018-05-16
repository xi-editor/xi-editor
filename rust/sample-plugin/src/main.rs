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

//! A sample plugin, intended as an illustartion and a template for plugin
//! developers.

extern crate xi_plugin_lib;
extern crate xi_core_lib as xi_core;
extern crate xi_rope;

use std::path::Path;

use xi_core::ConfigTable;
use xi_core::plugin_rpc::PluginEdit;
use xi_rope::rope::RopeDelta;
use xi_plugin_lib::{Plugin, ChunkCache, View, mainloop};

struct SamplePlugin;

// implementing the `Plugin` trait is the sole requirement of a plugin.
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

    fn update(&mut self, view: &mut View<Self::Cache>, _delta: Option<&RopeDelta>,
              _edit_type: String, _author: String) -> Option<PluginEdit> {
        eprintln!("edit in view {}", view.get_id());
        None
    }
}

fn main() {
    let mut plugin = SamplePlugin;
    mainloop(&mut plugin).unwrap();
}
