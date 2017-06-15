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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SyntaxDefinition {
    Plaintext, Markdown, Python, Rust, C, Go, Dart, Swift, Toml,
    Json, Yaml, Cpp, Objc, Shell, Ruby, Javascript, Java, Php,
    Perl,
}

impl Default for SyntaxDefinition {
    fn default() -> Self {
        SyntaxDefinition::Plaintext
    }
}

// TODO: these should also serialize as strings, probably using this as a guide:

impl SyntaxDefinition {
    pub fn new<'a, S: Into<Option<&'a str>>>(s: S) -> Self {
        use self::SyntaxDefinition::*;
        if let Some(s) = s.into() {
            match &*s.split('.').rev().nth(0).unwrap_or("").to_lowercase() {
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
        } else {
            Plaintext
        }
    }

    //TODO: this is not currently used.
    //We might want it for language server interop?
    /// Canonical language identifiers, used for serialization.
    /// https://code.visualstudio.com/docs/languages/identifiers
    pub fn identifier(&self) -> &str {
        use self::SyntaxDefinition::*;
        match *self {
            Rust => "rust",
            Markdown => "markdown",
            //TODO: :|
            Python => "python3",
            C => "c" ,
            Go => "go",
            Dart => "dart",
            Swift => "swift",
            Toml => "toml",
            Json => "json",
            Yaml => "yaml",
            Cpp => "cpp",
            Objc => "objective-c",
            Shell => "shellscript",
            Ruby => "ruby",
            Javascript => "javascript",
            Java => "java",
            Php => "php",
            Perl => "perl",
            Plaintext => "plaintext",
        }
    }
}

impl<S: AsRef<str>> From<S> for SyntaxDefinition {
    fn from(s: S) -> Self {
        SyntaxDefinition::new(s.as_ref())
    }
}

impl fmt::Display for SyntaxDefinition {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.identifier())
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
