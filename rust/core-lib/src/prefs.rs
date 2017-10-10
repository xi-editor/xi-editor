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

use std::{env, fs, io, fmt};
use std::io::Read;
use std::path::{PathBuf, Path};
use std::collections::HashMap;
use std::rc::Rc;

use toml;
use toml::value::{Value, Table};

use syntax::SyntaxDefinition;

static XI_CONFIG_DIR: &'static str = "XI_CONFIG_DIR";
static XI_CONFIG_FILE: &'static str = "preferences.xiconfig";
static XDG_CONFIG_HOME: &'static str = "XDG_CONFIG_HOME";

mod defaults {
    pub const BASE: &'static str = include_str!("../assets/defaults.toml");
}

/// Returns the location of the active config directory.
///
/// env vars are passed in as Option<&str> for easier testing.
fn config_dir_impl(xi_var: Option<&str>, xdg_var: Option<&str>) -> PathBuf {
    xi_var.map(PathBuf::from)
        .unwrap_or_else(|| {
            let mut xdg_config = xdg_var.map(PathBuf::from)
                .unwrap_or_else(|| {
                    env::var("HOME").map(PathBuf::from)
                        .map(|mut p| {
                            p.push(".config");
                            p
                        })
                        .expect("$HOME is required by POSIX")
                });
            xdg_config.push("xi");
            xdg_config
        })
}

fn get_config_dir() -> PathBuf {
    let xi_var = env::var(XI_CONFIG_DIR).ok();
    let xdg_var = env::var(XDG_CONFIG_HOME).ok();
    config_dir_impl(xi_var.as_ref().map(String::as_ref),
                    xdg_var.as_ref().map(String::as_ref))
}


fn init_config() -> ConfigSources {
    let config_dir = get_config_dir();
    if !config_dir.exists() {
        fs::create_dir(&config_dir);
    }
    let base_conf: Table = toml::from_str(defaults::BASE).unwrap();
    ConfigSources::new(base_conf, &config_dir)
}

fn load_config(path: &Path) -> Result<Table, ConfigError> {
    let mut file = fs::File::open(&path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let config: Table = toml::from_str(&contents)?;
    Ok(config)
}

fn load_syntax_configs(path: &Path) -> Vec<(PathBuf, Table)> {
    let mut result = Vec::new();
    let contents = match path.read_dir() {
        Ok(contents) => contents,
        Err(err) => {
            print_err!("Error reading config directory: {:?}", err);
            return result
        }
    };

    for item in contents {
        if let Ok(item) = item {
            let path = item.path();
            let skip = path.extension().map(|ext| ext != "xiconfig")
                .unwrap_or(true)
                || path.file_stem().map(|stem| stem == "preferences")
                .unwrap_or(true);
            if skip { continue }

            match load_config(&path) {
                Ok(prefs) => result.push((path, prefs)),
                Err(err) => print_err!("Error reading config file {:?}", &path),
            }
        }
    }
    result
}

#[derive(Debug)]
enum ConfigError {
    FileMissing,
    IoError(io::Error),
    Parse(toml::de::Error),
}

impl From<io::Error> for ConfigError {
    fn from(err: io::Error) -> ConfigError {
        ConfigError::IoError(err)
    }
}

impl From<toml::de::Error> for ConfigError {
    fn from(err: toml::de::Error) -> ConfigError {
        ConfigError::Parse(err)
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ConfigError::FileMissing => write!(f, "File missing"),
            ConfigError::IoError(ref err) => write!(f, "IO Error: {:?}", err),
            ConfigError::Parse(ref err) => write!(f, "TOML Error: {:?}", err),
        }
    }
}

pub struct ConfigSources {
    base: Rc<Table>,
    syntax: HashMap<SyntaxDefinition, Rc<Table>>,
    user: Rc<Table>,
    user_syntax: HashMap<SyntaxDefinition, Rc<Table>>,
}

pub struct ConfigSet {
    sources: Vec<Rc<Table>>
}

impl ConfigSources {
    fn new(base: Table, config_dir: &Path) -> Self {
        let user_pref_path = config_dir.join(XI_CONFIG_FILE);
        let user_prefs = match load_config(&user_pref_path) {
            Ok(prefs) => prefs,
            Err(err) => {
                print_err!("Error loading user prefs: {:?}", err);
                Table::new()
            }
        };

        let user_syntax = HashMap::new();
        let syntax_prefs = load_syntax_configs(&config_dir);
        if !syntax_prefs.is_empty() {
            panic!("actually using user syntax preferences is not implemented")
        }

        //TODO: keep a files-to-watch list

        ConfigSources {
            base: Rc::new(base),
            syntax: HashMap::new(),
            user: Rc::new(user_prefs),
            user_syntax: user_syntax,
        }
    }

    fn get_config(&self, syntax: SyntaxDefinition) -> ConfigSet {
        let mut sources = vec![self.base.clone(), self.user.clone()];
        if let Some(syntax_specific) = self.syntax.get(&syntax) {
            sources.push(syntax_specific.clone());
        }

        if let Some(syntax_specific) = self.user_syntax.get(&syntax) {
            sources.push(syntax_specific.clone());
        }

        sources.reverse();
        ConfigSet { sources }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_config() {
       let p = _get_config_dir(Some("custom/xi/conf"), None);
       assert_eq!(p, PathBuf::from("custom/xi/conf"));

       let p = _get_config_dir(Some("custom/xi/conf"), Some("/me/config"));
       assert_eq!(p, PathBuf::from("custom/xi/conf"));

       let p = _get_config_dir(None, Some("/me/config"));
       assert_eq!(p, PathBuf::from("/me/config/xi"));

       let p = _get_config_dir(None, None);
       let exp = env::var("HOME").map(PathBuf::from)
           .map(|mut p| { p.push(".config/xi"); p })
           .unwrap();
       assert_eq!(p, exp);
    }
}
