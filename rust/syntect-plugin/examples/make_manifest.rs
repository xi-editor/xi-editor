// Copyright 2018 The xi-editor Authors.
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

//! A simple tool that generates the syntect plugin's manifest.

extern crate syntect;
extern crate toml;
extern crate xi_core_lib as xi_core;

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use syntect::parsing::{SyntaxDefinition, SyntaxSet};
use toml::Value;
use xi_core::plugin_manifest::*;
use xi_core::LanguageDefinition;

const OUT_FILE_NAME: &str = "generated_manifest.toml";

/// Extracts the name and version from Cargo.toml
fn parse_name_and_version() -> Result<(String, String), io::Error> {
    eprintln!("exe: {:?}", ::std::env::current_exe());
    let path = PathBuf::from("./Cargo.toml");
    let toml_str = fs::read_to_string(path)?;
    let value = toml_str.parse::<Value>().unwrap();
    let package_table = value["package"].as_table().unwrap();
    let name = package_table["name"].as_str().unwrap().to_string();
    let version = package_table["version"].as_str().unwrap().to_string();
    Ok((name, version))
}

fn main() -> Result<(), io::Error> {
    let syntax_set = SyntaxSet::load_defaults_newlines();
    let lang_defs = syntax_set
        .syntaxes()
        .iter()
        .filter(|syntax| !syntax.file_extensions.is_empty())
        .map(lang_from_syn)
        .collect::<Vec<_>>();

    let (name, version) = parse_name_and_version()?;
    let exec_path = PathBuf::from(format!("./bin/{}", &name));

    let mani = PluginDescription {
        name,
        version,
        scope: PluginScope::Global,
        exec_path,
        activations: vec![PluginActivation::Autorun],
        commands: vec![],
        languages: lang_defs,
    };

    let toml_str = toml::to_string(&mani).unwrap();
    let file_path = Path::new(OUT_FILE_NAME);
    let mut f = File::create(file_path)?;
    f.write_all(toml_str.as_ref())
}

fn lang_from_syn<'a>(src: &'a SyntaxDefinition) -> LanguageDefinition {
    LanguageDefinition {
        name: src.name.as_str().into(),
        extensions: src.file_extensions.clone(),
        first_line_match: src.first_line_match.clone(),
        scope: src.scope.to_string(),
        default_config: None,
    }
}
