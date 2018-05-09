// Copyright 2018 Google LLC
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
extern crate  xi_core_lib as xi_core;

use std::io::Write;
use std::path::PathBuf;
use std::fs::File;

use xi_core::plugin_manifest::*;
use xi_core::LanguageDefinition;
use syntect::parsing::{SyntaxSet, SyntaxDefinition};


fn main() {
    let syntax_set = SyntaxSet::load_defaults_newlines();
    let lang_defs = syntax_set.syntaxes().iter()
        .filter(|syntax| !syntax.file_extensions.is_empty())
        .map(lang_from_syn)
        .collect::<Vec<_>>();

    let mani = PluginDescription {
        name: "syntect".into(),
        version: "0.1".into(),
        scope: PluginScope::Global,
        exec_path: PathBuf::from("./bin/xi-syntect-plugin"),
        activations: vec![PluginActivation::Autorun],
        commands: vec![],
        languages: lang_defs,
    };

	let toml_str = toml::to_string(&mani).unwrap();
	let mut f = File::create("xi_manifest.toml").unwrap();
    f.write_all(toml_str.as_ref()).unwrap();
}

fn lang_from_syn<'a>(src: &'a SyntaxDefinition) -> LanguageDefinition {
    LanguageDefinition {
        name: src.name.clone(),
        extensions: src.file_extensions.clone(),
        first_line_match: src.first_line_match.clone(),
        scope: src.scope.to_string(),
        default_config: None,
    }
}

