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

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read};
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use toml;

use super::{PluginName, PluginDescription};
use config::table_from_toml_str;
use syntax::{LanguageDefinition, Languages};

/// A catalog of all available plugins.
#[derive(Debug, Clone, Default)]
pub struct PluginCatalog {
    items: HashMap<PluginName, Arc<PluginDescription>>,
    locations: HashMap<PathBuf, Arc<PluginDescription>>,
}

/// Errors that can occur while trying to load a plugin.
#[derive(Debug)]
pub enum PluginLoadError {
    Io(io::Error),
    /// Malformed manifest
    Parse(toml::de::Error),
}

#[allow(dead_code)]
impl <'a>PluginCatalog {
    pub fn reload_from_paths(&mut self, paths: &[PathBuf]) {
        self.items.clear();
        self.locations.clear();
        let all_manifests = find_all_manifests(paths);
        for manifest_path in all_manifests.iter() {
            match load_manifest(manifest_path) {
                Err(e) => eprintln!("error loading plugin {:?}", e),
                Ok(manifest) => {
                    let manifest = Arc::new(manifest);
                    self.items.insert(manifest.name.clone(), manifest.clone());
                    self.locations.insert(manifest_path.clone(), manifest);
                }
            }
        }
    }

    pub fn make_languages_map(&self) -> Languages {
        let all_langs = self.items.values()
            .flat_map(|plug| plug.languages.iter().cloned())
            .collect::<Vec<_>>();
        Languages::new(all_langs.as_slice())
    }

    ///// Loads plugins from the user's search paths
    //pub fn from_paths(paths: Vec<PathBuf>) -> Self {
        //let plugins = paths.iter()
            //.flat_map(|path| {
                //match load_plugins(path) {
                    //Ok(plugins) => plugins,
                    //Err(err) => {
                        //eprintln!("error loading plugins from {:?}, error:\n{:?}",
                                   //path, err);
                        //Vec::new()
                    //}
                //}
            //})
            //.collect::<Vec<_>>();
        //PluginCatalog::new(&plugins)
    //}

    //pub fn new(plugins: &[PluginDescription]) -> Self {
        //let mut items = HashMap::with_capacity(plugins.len());
        //for plugin in plugins {
            //if items.contains_key(&plugin.name) {
                //eprintln!("Duplicate plugin name.\n 1: {:?}\n 2: {:?}",
                           //plugin, items.get(&plugin.name));
                //continue
            //}
            //items.insert(plugin.name.to_owned(), plugin.to_owned());
        //}
        //PluginCatalog { items }
    //}

    /// Returns an iterator over all plugins in the catalog, in arbitrary order.
    pub fn iter(&'a self) -> Box<Iterator<Item=&'a PluginDescription> + 'a> {
       Box::new(self.items.values())
    }

    /// Returns an iterator over all plugin names in the catalog,
    /// in arbitrary order.
    pub fn iter_names(&'a self) -> Box<Iterator<Item=&'a PluginName> + 'a> {
        Box::new(self.items.keys())
    }

    /// Returns a reference to the named plugin if it exists in the catalog.
    pub fn get_named(&self, plugin_name: &str) -> Option<Arc<PluginDescription>> {
        self.items.get(plugin_name).map(Arc::clone)
    }

    /// Returns all PluginDescriptions matching some predicate
    pub fn filter<F>(&self, predicate: F) -> Vec<&PluginDescription>
    where F: Fn(&PluginDescription) -> bool {
        self.iter()
            .filter(|item| predicate(item))
            .collect::<Vec<_>>()
    }

    pub fn get_lang_defs(&self) -> Vec<LanguageDefinition> {
        self.iter()
            .flat_map(|p| p.languages.iter().cloned())
            .collect()
    }
}

fn load_plugins(plugin_dir: &Path)
    -> Result<Vec<PluginDescription>, PluginLoadError> {
    // if this is just a path to a single plugin, just return that
    let manif_path = plugin_dir.join("manifest");
    if manif_path.exists() {
        let manifest = load_manifest(&manif_path)?;
        return Ok(vec![manifest]);
    }

    let mut plugins = Vec::new();
    for path in plugin_dir.read_dir()? {
        let path = path?;
        let path = path.path();
        if !path.is_dir() { continue }
        let manif_path = path.join("manifest.toml");
        if !manif_path.exists() { continue }
        match load_manifest(&manif_path) {
            Ok(manif) => plugins.push(manif),
            Err(err) => eprintln!("Error reading manifest {:?}, error:\n{:?}",
                                   &manif_path, err),
        }
    }
    Ok(plugins)
}

fn find_all_manifests(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut manifest_paths = Vec::new();
    for path in paths.iter() {
        let manif_path = path.join("manifest.toml");
        if manif_path.exists() {
            manifest_paths.push(manif_path);
            continue;
        }

        let contained_manifests = path.read_dir()
            .map(|dir|
                 dir.flat_map(|item| item.map(|p| p.path()).ok())
                 .map(|dir| dir.join("manifest.toml"))
                 .filter(|f| f.exists())
                 .for_each(|f| manifest_paths.push(f.to_owned())));
    }
    manifest_paths
}

fn load_manifest(path: &Path) -> Result<PluginDescription, PluginLoadError> {
    let mut file = fs::File::open(&path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let mut manifest: PluginDescription = toml::from_str(&contents)?;
    // normalize relative paths
    if manifest.exec_path.starts_with("./") {
        manifest.exec_path = path.parent()
            .unwrap()
            .join(manifest.exec_path)
            .canonicalize()?;
    }

    for lang in manifest.languages.iter_mut() {
        let lang_config_path = path.parent().unwrap()
            .join(&lang.name.0)
            .with_extension("xiconfig");
        if !lang_config_path.exists() { continue; }
        let lang_defaults = fs::read_to_string(&lang_config_path)?;
        let lang_defaults = table_from_toml_str(&lang_defaults)?;
        lang.default_config = Some(lang_defaults);
    }
    Ok(manifest)
}

impl From<io::Error> for PluginLoadError {
    fn from(err: io::Error) -> PluginLoadError {
        PluginLoadError::Io(err)
    }
}

impl From<toml::de::Error> for PluginLoadError {
    fn from(err: toml::de::Error) -> PluginLoadError {
        PluginLoadError::Parse(err)
    }
}
