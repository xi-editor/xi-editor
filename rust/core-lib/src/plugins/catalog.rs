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

use std::collections::{HashMap, hash_map};

use super::{PluginName, PluginDescription};
use super::manifest::debug_plugins;

/// A catalog of all available plugins.
pub struct PluginCatalog {
    items: HashMap<PluginName, PluginDescription>,
}

impl <'a>PluginCatalog {
    /// For use during development: returns the debug plugins
    pub fn debug() -> Self {
        PluginCatalog::new(&debug_plugins())
    }

    pub fn new(plugins: &[PluginDescription]) -> Self {
        let mut items = HashMap::with_capacity(plugins.len());
        for plugin in plugins {
            if items.contains_key(&plugin.name) {
                print_err!("Duplicate plugin name.\n 1: {:?}\n 2: {:?}",
                           plugin, items.get(&plugin.name));
                continue
            }
            items.insert(plugin.name.to_owned(), plugin.to_owned());
        }
        PluginCatalog { items }
    }

    pub fn iter(&'a self) -> hash_map::Values<PluginName, PluginDescription> {
       self.items.values()
    }

    pub fn iter_names(&'a self) -> hash_map::Keys<PluginName, PluginDescription> {
        self.items.keys()
    }

    pub fn get_named(&self, plugin_name: &str) -> Option<&PluginDescription> {
        self.items.get(plugin_name)
    }

    /// Returns all PluginDescriptions matching some predicate
    pub fn filter<F>(&self, predicate: F) -> Vec<&PluginDescription>
    where F: Fn(&PluginDescription) -> bool {
        self.iter()
            .filter(|item| predicate(item))
            .collect::<Vec<_>>()
    }
}
