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

//! Very basic syntax detection.

use std::borrow::Borrow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use config::Table;

/// The canonical identifier for a particular `LanguageDefinition`.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct LanguageId(Arc<String>);

/// Describes a `LanguageDefinition`. Although these are provided by plugins,
/// they are a fundamental concept in core, used to determine things like
/// plugin activations and active user config tables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageDefinition {
    pub name: LanguageId,
    pub extensions: Vec<String>,
    pub first_line_match: Option<String>,
    pub scope: String,
    #[serde(skip)]
    pub default_config: Option<Table>,
}

/// A repository of all loaded `LanguageDefinition`s.
#[derive(Debug, Default)]
pub struct Languages {
    named: HashMap<LanguageId, Arc<LanguageDefinition>>,
    extensions: HashMap<String, Arc<LanguageDefinition>>,
}

impl Languages {
    pub fn new(language_defs: &[LanguageDefinition]) -> Self {
        let mut named = HashMap::new();
        let mut extensions = HashMap::new();
        for lang in language_defs.iter() {
            let lang_arc = Arc::new(lang.clone());
            named.insert(lang.name.clone(), lang_arc.clone());
            for ext in lang.extensions.iter() {
                extensions.insert(ext.clone(), lang_arc.clone());
            }
        }
        Languages { named, extensions }
    }

    pub fn language_for_path(&self, path: &Path) -> Option<Arc<LanguageDefinition>> {
        path.extension()
            .and_then(|ext| self.extensions.get(ext.to_str().unwrap_or_default()))
            .map(Arc::clone)
    }

    pub fn language_for_name<S>(&self, name: S) -> Option<Arc<LanguageDefinition>>
        where S: AsRef<str>
    {
        self.named.get(name.as_ref()).map(Arc::clone)
    }

    /// Returns a Vec of any `LanguageDefinition`s which exist
    /// in `self` but not `other`.
    pub fn difference(&self, other: &Languages) -> Vec<Arc<LanguageDefinition>> {
        self.named.iter()
            .filter(|(k, _)| !other.named.contains_key(*k))
            .map(|(_, v)| v.clone())
            .collect()
    }

    pub fn iter<'a>(&'a self) -> impl Iterator<Item=&'a Arc<LanguageDefinition>> {
        self.named.values()
    }
}

impl AsRef<str> for LanguageId {
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}

// let's us use &str to query a HashMap with `LanguageId` keys
impl Borrow<str> for LanguageId {
    fn borrow(&self) -> &str {
        &self.0.as_ref()
    }
}

impl<'a> From<&'a str> for LanguageId {
    fn from(src: &'a str) -> LanguageId {
        LanguageId(Arc::new(src.into()))
    }
}

// for testing
#[cfg(test)]
impl LanguageDefinition {
    pub(crate) fn simple(name: &str, exts: &[&str],
                         scope: &str, config: Option<Table>) -> Self
    {
        LanguageDefinition {
            name: name.into(),
            extensions: exts.iter().map(|s| (*s).into()).collect(),
            first_line_match: None,
            scope: scope.into(),
            default_config: config,
        }
    }
}
