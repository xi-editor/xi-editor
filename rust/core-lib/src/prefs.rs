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

use config::{self, Source, Value, FileFormat};


static XI_CONFIG_DIR: &'static str = "XI_CONFIG_DIR";
static XDG_CONFIG_HOME: &'static str = "XDG_CONFIG_HOME";
/// A client can use this to pass a path to bundled plugins
static XI_SYS_PLUGIN_PATH: &'static str = "XI_SYS_PLUGIN_PATH";
static XI_CONFIG_FILE_NAME: &'static str = "preferences.xiconfig";

/// Namespace for various default settings.
#[allow(unused)]
mod defaults {
    use super::*;
    pub const BASE: &'static str = include_str!("../assets/defaults.toml");
    pub const WINDOWS: &'static str = include_str!("../assets/windows.toml");

    fn platform_overrides() -> Option<Table> {
        #[cfg(target_os = "windows")]
        { return Some(load(WINDOWS)) }
        None
    }

    pub fn platform_defaults() -> Table {
        let mut base = load(BASE);
        if let Some(mut overrides) = platform_overrides() {
            for (k, v) in overrides.drain() {
                base.insert(k, v);
            }
        }
        base
    }

    fn load(default: &str) -> Table {
        config::File::from_str(default, config::FileFormat::Toml)
            .collect()
            .expect("default configs must load")
    }
}

pub type Table = HashMap<String, Value>;

pub struct ConfigManager {
    config_dir: PathBuf,
    /// The default config
    base: Table,
    /// The user's custom config
    user: Table,
    /// A cache of the merged configs
    cache: Table,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A container for all user-modifiable settings.
pub struct Config {
    pub newline: String,
    pub tab_size: usize,
    pub translate_tabs_to_spaces: bool,
    pub plugin_search_path: Vec<PathBuf>,
}

impl ConfigManager {
    fn new<P: AsRef<Path>>(config_dir: P, user_config: Table) -> Self {
        let base_config = defaults::platform_defaults();
        let mut conf = ConfigManager {
            config_dir: config_dir.as_ref().to_owned(),
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

    /// Generates a snapshot of the currently loaded configuration.
    pub fn get_config(&self) -> Config {
        let settings: Value = self.cache.clone().into();
        let mut settings: Config = settings.try_into().unwrap();
        // relative entries in plugin search path should be relative to
        // the config directory.
        settings.plugin_search_path = settings.plugin_search_path
            .iter()
            .map(|p| self.config_dir.join(p))
            .collect();
        // If present, append the location of plugins bundled by client
        if let Ok(sys_path) = env::var(XI_SYS_PLUGIN_PATH) {
            print_err!("including client bundled plugins from {}", &sys_path);
            settings.plugin_search_path.push(sys_path.into());
        }
        settings
    }
}

impl Default for ConfigManager {
    fn default() -> ConfigManager {
        let path = get_config_dir();
        let config_path = path.join(XI_CONFIG_FILE_NAME);
        let user_config: config::File<_> = config_path.into();
        let user_config = user_config
            .format(FileFormat::Toml)
            .collect()
            .map_err(|e| print_err!("Error reading config: {:?}", e))
            .unwrap_or_default();

        ConfigManager::new(&path, user_config)
    }
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
        let manager = ConfigManager::new("BASE_PATH", Table::default());
        let config = manager.get_config();
        assert_eq!(config.tab_size, 4);
        assert_eq!(config.plugin_search_path, vec![PathBuf::from("BASE_PATH/plugins")])
    }

    #[test]
    fn test_overrides() {
        let user_config = r#"tab_size = 42"#;
        let user_config = config::File::from_str(user_config, FileFormat::Toml)
            .collect()
            .unwrap();
        let manager = ConfigManager::new("", user_config);
        let config = manager.get_config();
        assert_eq!(config.tab_size, 42);
    }
}
