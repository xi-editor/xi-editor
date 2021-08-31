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
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

use crate::config::Table;

/// The canonical identifier for a particular `LanguageDefinition`.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[allow(clippy::rc_buffer)] // suppress clippy;  TODO consider addressing
                            // the warning by changing String to str
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
    // NOTE: BTreeMap is used for sorting the languages by name alphabetically
    named: BTreeMap<LanguageId, Arc<LanguageDefinition>>,
    extensions: HashMap<String, Arc<LanguageDefinition>>,
}

impl Languages {
    pub fn new(language_defs: &[LanguageDefinition]) -> Self {
        let mut named = BTreeMap::new();
        let mut extensions = HashMap::new();
        for lang in language_defs.iter() {
            let lang_arc = Arc::new(lang.clone());
            named.insert(lang.name.clone(), lang_arc.clone());
            for ext in &lang.extensions {
                extensions.insert(ext.clone(), lang_arc.clone());
            }
        }
        Languages { named, extensions }
    }

    pub fn language_for_path(&self, path: &Path) -> Option<Arc<LanguageDefinition>> {
        path.extension()
            .or_else(|| path.file_name())
            .and_then(|ext| self.extensions.get(ext.to_str().unwrap_or_default()))
            .map(Arc::clone)
    }

    pub fn language_for_name<S>(&self, name: S) -> Option<Arc<LanguageDefinition>>
    where
        S: AsRef<str>,
    {
        self.named.get(name.as_ref()).map(Arc::clone)
    }

    /// Returns a Vec of any `LanguageDefinition`s which exist
    /// in `self` but not `other`.
    pub fn difference(&self, other: &Languages) -> Vec<Arc<LanguageDefinition>> {
        self.named
            .iter()
            .filter(|(k, _)| !other.named.contains_key(*k))
            .map(|(_, v)| v.clone())
            .collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<LanguageDefinition>> {
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
        self.0.as_ref()
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
    pub(crate) fn simple(name: &str, exts: &[&str], scope: &str, config: Option<Table>) -> Self {
        LanguageDefinition {
            name: name.into(),
            extensions: exts.iter().map(|s| (*s).into()).collect(),
            first_line_match: None,
            scope: scope.into(),
            default_config: config,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn language_for_path() {
        let ld_rust = LanguageDefinition {
            name: LanguageId::from("Rust"),
            extensions: vec![String::from("rs")],
            scope: String::from("source.rust"),
            first_line_match: None,
            default_config: None,
        };
        let ld_commit_msg = LanguageDefinition {
            name: LanguageId::from("Git Commit"),
            extensions: vec![
                String::from("COMMIT_EDITMSG"),
                String::from("MERGE_MSG"),
                String::from("TAG_EDITMSG"),
            ],
            scope: String::from("text.git.commit"),
            first_line_match: None,
            default_config: None,
        };
        let languages = Languages::new(&[ld_rust.clone(), ld_commit_msg.clone()]);

        assert_eq!(
            ld_rust.name,
            languages.language_for_path(Path::new("/path/test.rs")).unwrap().name
        );
        assert_eq!(
            ld_commit_msg.name,
            languages.language_for_path(Path::new("/path/COMMIT_EDITMSG")).unwrap().name
        );
        assert_eq!(
            ld_commit_msg.name,
            languages.language_for_path(Path::new("/path/MERGE_MSG")).unwrap().name
        );
        assert_eq!(
            ld_commit_msg.name,
            languages.language_for_path(Path::new("/path/TAG_EDITMSG")).unwrap().name
        );
    }
}
