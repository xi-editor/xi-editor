// Copyright 2017 The xi-editor Authors.
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

//! Keeping track of available plugins.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{PluginDescription, PluginName};
use crate::config::table_from_toml_str;
use crate::syntax::Languages;

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
impl<'a> PluginCatalog {
    /// Loads any plugins discovered in these paths, replacing any existing
    /// plugins.
    pub fn reload_from_paths(&mut self, paths: &[PathBuf]) {
        self.items.clear();
        self.locations.clear();
        self.load_from_paths(paths);
    }

    /// Loads plugins from paths and adds them to existing plugins.
    pub fn load_from_paths(&mut self, paths: &[PathBuf]) {
        let all_manifests = find_all_manifests(paths);
        for manifest_path in &all_manifests {
            match load_manifest(manifest_path) {
                Err(e) => warn!("error loading plugin {:?}", e),
                Ok(manifest) => {
                    info!("loaded {}", manifest.name);
                    let manifest = Arc::new(manifest);
                    self.items.insert(manifest.name.clone(), manifest.clone());
                    self.locations.insert(manifest_path.clone(), manifest);
                }
            }
        }
    }

    pub fn make_languages_map(&self) -> Languages {
        let all_langs =
            self.items.values().flat_map(|plug| plug.languages.iter().cloned()).collect::<Vec<_>>();
        Languages::new(all_langs.as_slice())
    }

    /// Returns an iterator over all plugins in the catalog, in arbitrary order.
    pub fn iter(&'a self) -> impl Iterator<Item = Arc<PluginDescription>> + 'a {
        self.items.values().cloned()
    }

    /// Returns an iterator over all plugin names in the catalog,
    /// in arbitrary order.
    pub fn iter_names(&'a self) -> impl Iterator<Item = &'a PluginName> {
        self.items.keys()
    }

    /// Returns the plugin located at the provided file path.
    pub fn get_from_path(&self, path: &PathBuf) -> Option<Arc<PluginDescription>> {
        self.items
            .values()
            .find(|&v| v.exec_path.to_str().unwrap().contains(path.to_str().unwrap()))
            .cloned()
    }

    /// Returns a reference to the named plugin if it exists in the catalog.
    pub fn get_named(&self, plugin_name: &str) -> Option<Arc<PluginDescription>> {
        self.items.get(plugin_name).map(Arc::clone)
    }

    /// Removes the named plugin.
    pub fn remove_named(&mut self, plugin_name: &str) {
        self.items.remove(plugin_name);
    }
}

fn find_all_manifests(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut manifest_paths = Vec::new();
    for path in paths.iter() {
        let manif_path = path.join("manifest.toml");
        if manif_path.exists() {
            manifest_paths.push(manif_path);
            continue;
        }

        let result = path.read_dir().map(|dir| {
            dir.flat_map(|item| item.map(|p| p.path()).ok())
                .map(|dir| dir.join("manifest.toml"))
                .filter(|f| f.exists())
                .for_each(|f| manifest_paths.push(f))
        });
        if let Err(e) = result {
            error!("error reading plugin path {:?}, {:?}", path, e);
        }
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
        manifest.exec_path = path.parent().unwrap().join(manifest.exec_path).canonicalize()?;
    }

    for lang in &mut manifest.languages {
        let lang_config_path =
            path.parent().unwrap().join(&lang.name.as_ref()).with_extension("toml");
        if !lang_config_path.exists() {
            continue;
        }
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
