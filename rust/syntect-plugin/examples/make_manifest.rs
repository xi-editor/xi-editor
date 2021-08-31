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

use crate::xi_core::plugin_manifest::*;
use crate::xi_core::LanguageDefinition;
use syntect::dumps::dump_to_file;
use syntect::parsing::{SyntaxReference, SyntaxSetBuilder};
use toml::Value;

const OUT_FILE_NAME: &str = "manifest.toml";

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
    let package_dir = "syntect-resources/Packages";
    let packpath = "assets/default.packdump";
    let metasource = "syntect-resources/DefaultPackage";
    let metapath = "assets/default_meta.packdump";

    let mut builder = SyntaxSetBuilder::new();
    builder.add_plain_text_syntax();
    builder.add_from_folder(package_dir, true).unwrap();
    builder.add_from_folder(metasource, false).unwrap();
    let syntax_set = builder.build();

    dump_to_file(&syntax_set, packpath).unwrap();
    dump_to_file(&syntax_set.metadata(), metapath).unwrap();

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

fn lang_from_syn(src: &SyntaxReference) -> LanguageDefinition {
    let mut extensions = src.file_extensions.clone();

    // add support for .xiconfig
    if extensions.contains(&String::from("toml")) {
        extensions.push(String::from("xiconfig"));
    }

    LanguageDefinition {
        name: src.name.as_str().into(),
        extensions,
        first_line_match: src.first_line_match.clone(),
        scope: src.scope.to_string(),
        default_config: None,
    }
}
