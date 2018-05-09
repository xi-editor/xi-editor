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

//! Very basic syntax detection.

use std::fmt;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde::de::{value, Deserialize, IntoDeserializer};
use serde_json;

use config::Table;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct LanguageId(pub(crate) String);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageDefinition {
    pub name: LanguageId,
    pub extensions: Vec<String>,
    pub first_line_match: Option<String>,
    pub scope: String,
    #[serde(skip)]
    pub default_config: Option<Table>,
}

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
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum SyntaxDefinition {
    Plaintext, Markdown, Python, Rust, C, Go, Dart, Swift, Toml,
    Json, Yaml, Cpp, Objc, Shell, Ruby, Javascript, Java, Php,
    Perl, Makefile,
}

impl Default for SyntaxDefinition {
    fn default() -> Self {
        SyntaxDefinition::Plaintext
    }
}

//FIXME: this should be Into<SyntaxDefinition> for AsRef<Path>, or something
impl SyntaxDefinition {
    pub fn new<'a, S: Into<Option<&'a str>>>(s: S) -> Self {
        use self::SyntaxDefinition::*;
        let s = s.into().unwrap_or("").to_lowercase();
        if s == "makefile" { return Makefile }

        match &*s.split('.').rev().nth(0).unwrap_or("") {
            "rs" => Rust,
            "md" | "mdown" => Markdown,
            "py" => Python,
            "c" | "h" => C,
            "go" => Go,
            "dart" => Dart,
            "swift" => Swift,
            "toml" => Toml,
            "json" => Json,
            "yaml" => Yaml,
            "cc" => Cpp,
            "m" => Objc,
            "sh" | "zsh" => Shell,
            "rb" => Ruby,
            "js" => Javascript,
            "java" | "jav" => Java,
            "php" => Php,
            "pl" => Perl,
            _ => Plaintext,
        }
    }

    /// Attempt to parse a name into a `SyntaxDefinition`.
    ///
    /// Note:
    /// This uses serde deserialization under the hood; this governs what
    /// names are expected to work.
    pub fn try_from_name<S: AsRef<str>>(name: S) -> Option<Self> {
        let r: Result<Self, value::Error> = Self::deserialize(
            name.as_ref().into_deserializer());
        r.ok()
    }
}

impl<S: AsRef<str>> From<S> for SyntaxDefinition {
    fn from(s: S) -> Self {
        SyntaxDefinition::new(s.as_ref())
    }
}

impl fmt::Display for SyntaxDefinition {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", serde_json::to_string(self).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_syntax() {
        assert_eq!(SyntaxDefinition::from("plugins.rs"), SyntaxDefinition::Rust);
        assert_eq!(SyntaxDefinition::from("plugins.py"), SyntaxDefinition::Python);
        assert_eq!(SyntaxDefinition::from("header.h"), SyntaxDefinition::C);
        assert_eq!(SyntaxDefinition::from("main.ada"), SyntaxDefinition::Plaintext);
        assert_eq!(SyntaxDefinition::from("build"), SyntaxDefinition::Plaintext);
        assert_eq!(SyntaxDefinition::from("build.test.sh"), SyntaxDefinition::Shell);
    }
}
