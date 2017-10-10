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

use std::env;
use std::path::{PathBuf, Path};
use std::collections::HashMap;

use config::{self, Source, Value, ConfigError};


static XI_CONFIG_DIR: &'static str = "XI_CONFIG_DIR";
static XDG_CONFIG_HOME: &'static str = "XDG_CONFIG_HOME";
static XI_CONFIG_FILE_NAME: &'static str = "preferences.xiconfig";

/// Namespace for various default settings.
mod defaults {
    pub const BASE: &'static str = include_str!("../assets/defaults.toml");
}

pub type Table = HashMap<String, Value>;

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

pub struct ConfigManager {
    /// The default config
    base: Table,
    /// The user's custom config
    user: Table,
    /// A cache of the merged configs
    cache: Table,
}

pub struct Config(Table);

impl ConfigManager {
    fn new(config_dir: &Path) -> Self {
        let base_config = config::File::from_str(&defaults::BASE,
                                                 config::FileFormat::Toml)
            .collect()
            .expect("base configuration settings must load.");
        let config_path = config_dir.join(XI_CONFIG_FILE_NAME);
        let user_config: config::File<_> = config_path.into();
        let user_config = user_config
            .collect()
            .map_err(|e| print_err!("Error reading config: {:?}", e))
            .unwrap_or_default();

        let mut conf = ConfigManager {
            base: base_config,
            user: user_config,
            cache: Table::default(),
        };
        conf.rebuild();
        conf
    }

    fn rebuild(&mut self) {
        let mut cache = self.base.clone();
        for (k, v) in self.user.iter() {
            cache.insert(k.to_owned(), v.clone());
        }
        self.cache = cache;
    }

    //TODO: this should accept a 'syntax' argument eventually
    pub fn get_config(&self) -> Config {
        Config(self.cache.clone())
    }
}

impl Default for ConfigManager {
    fn default() -> ConfigManager {
        let path = get_config_dir();
        ConfigManager::new(&path)
    }
}

impl Config {
    fn get(&self, key: &str) -> Result<Value, ConfigError> {
        self.0.get(key).map(|v| v.clone())
            .ok_or(ConfigError::NotFound(key.to_owned()))
    }

    pub fn get_str(&self, key: &str) -> Result<String, ConfigError> {
        self.get(key).and_then(Value::into_str)
    }

    pub fn get_int(&self, key: &str) -> Result<i64, ConfigError> {
        self.get(key).and_then(Value::into_int)
    }

    pub fn get_float(&self, key: &str) -> Result<f64, ConfigError> {
        self.get(key).and_then(Value::into_float)
    }

    pub fn get_bool(&self, key: &str) -> Result<bool, ConfigError> {
        self.get(key).and_then(Value::into_bool)
    }

    pub fn get_table(&self, key: &str)
        -> Result<HashMap<String, Value>, ConfigError> {
        self.get(key).and_then(Value::into_table)
    }

    pub fn get_array(&self, key: &str) -> Result<Vec<Value>, ConfigError> {
        self.get(key).and_then(Value::into_array)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_config() {
       let p = config_dir_impl(Some("custom/xi/conf"), None);
       assert_eq!(p, PathBuf::from("custom/xi/conf"));

       let p = config_dir_impl(Some("custom/xi/conf"), Some("/me/config"));
       assert_eq!(p, PathBuf::from("custom/xi/conf"));

       let p = config_dir_impl(None, Some("/me/config"));
       assert_eq!(p, PathBuf::from("/me/config/xi"));

       let p = config_dir_impl(None, None);
       let exp = env::var("HOME").map(PathBuf::from)
           .map(|mut p| { p.push(".config/xi"); p })
           .unwrap();
       assert_eq!(p, exp);
    }

    #[test]
    fn test_defaults() {
        let manager = ConfigManager::default();
        let config = manager.get_config();
        assert_eq!(config.get_int("tab_size").unwrap(), 4);
        assert!(config.get_int("font_face").is_err());
        let plug_path = config.get_array("plugin_search_path").unwrap();
        assert_eq!(plug_path.len(), 1);
    }
}
