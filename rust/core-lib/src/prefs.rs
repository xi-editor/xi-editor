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
use std::collections::{HashMap, HashSet};

use config::{self, Source, Value, FileFormat};
use syntax::SyntaxDefinition;
use tabs::BufferIdentifier;


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
    pub const YAML: &'static str = include_str!("../assets/yaml.toml");
    pub const MAKEFILE: &'static str = include_str!("../assets/makefile.toml");

    pub fn platform_defaults() -> Table {
        let mut base = load(BASE);
        if let Some(mut overrides) = platform_overrides() {
            for (k, v) in overrides.drain() {
                base.insert(k, v);
            }
        }
        base
    }

    pub fn syntax_defaults() -> HashMap<SyntaxDefinition, Table>  {
        let mut configs = HashMap::new();
        configs.insert(SyntaxDefinition::Yaml, load(YAML));
        configs.insert(SyntaxDefinition::Makefile, load(MAKEFILE));
        configs
    }

    fn platform_overrides() -> Option<Table> {
        #[cfg(target_os = "windows")]
        { return Some(load(WINDOWS)) }
        None
    }

    fn load(default: &str) -> Table {
        config::File::from_str(default, config::FileFormat::Toml)
            .collect()
            .expect("default configs must load")
    }
}

pub type Table = HashMap<String, Value>;

/// Represents the common pattern of default settings masked by
/// user settings.
pub struct ConfigPair {
    /// A static default configuration, which will never change.
    base: Option<Table>,
    /// A variable, user provided configuration. Items here take
    /// precedence over items in `base`.
    user: Option<Table>,
    /// A snapshot of base + user.
    cache: Table,
}

pub struct ConfigManager {
    config_dir: PathBuf,
    /// The defaults, and any base user overrides
    defaults: ConfigPair,
    /// default per-syntax configs
    syntax_specific: HashMap<SyntaxDefinition, ConfigPair>,
    /// per-session overrides
    overrides: HashMap<BufferIdentifier, ConfigPair>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A container for all user-modifiable settings.
pub struct Config {
    pub newline: String,
    pub tab_size: usize,
    pub translate_tabs_to_spaces: bool,
    pub plugin_search_path: Vec<PathBuf>,
}

impl ConfigPair {
    fn new<T1, T2>(base: T1, user: T2) -> Self
        where T1: Into<Option<Table>>,
              T2: Into<Option<Table>>,
    {
        let base = base.into();
        let user = user.into();
        let cache = Table::new();
        let mut conf = ConfigPair { base, user, cache };
        conf.rebuild();
        conf
    }

    fn rebuild(&mut self) {
        let mut cache = self.base.clone().unwrap_or_default();
        if let Some(ref user) = self.user {
            for (k, v) in user.iter() {
                cache.insert(k.to_owned(), v.clone());
            }
        }
        self.cache = cache;
    }

    /// Manually sets a key/value pair in one of `base` or `user`.
    ///
    /// Note: this is only intended to be used internally, when handling
    /// overrides.
    fn set_override<K, V>(&mut self, key: K, value: V, from_user: bool)
        where K: AsRef<str>,
              V: Into<Value>,
    {
        let key: String = key.as_ref().to_owned();
        let value = value.into();
        {
            let table = if from_user {
                self.user.get_or_insert(Table::new())
            } else {
                self.base.get_or_insert(Table::new())
            };
            table.insert(key, value);
        }
        self.rebuild();
    }

    /// Returns a new `Table`, with the values of `other`
    /// inserted into a copy of `self.cache`.
    fn merged_with(&self, other: &ConfigPair) -> Table {
        let mut result = self.cache.clone();
        merge_tables(&mut result, &other.cache);
        result
    }
}

impl ConfigManager {
    fn new<P>(config_dir: P, user_config: Table,
              mut user_syntax: HashMap<SyntaxDefinition, Table>) -> Self
        where P: AsRef<Path>
    {
        let config_dir = config_dir.as_ref().to_owned();
        let defaults = ConfigPair::new(defaults::platform_defaults(),
                                       user_config);

        let mut syntax_defaults = defaults::syntax_defaults();
        // all syntaxes that have system and/or user settings
        let union_syntax_defs = syntax_defaults.keys()
            .chain(user_syntax.keys())
            .cloned()
            .collect::<HashSet<_>>();

        let syntax_specific = union_syntax_defs.iter()
            .map(|def| {
                let pair = ConfigPair::new(syntax_defaults.remove(def),
                                           user_syntax.remove(def));
                (def.to_owned(), pair)
            })
            .collect::<HashMap<SyntaxDefinition, ConfigPair>>();
        let overrides = HashMap::new();

        ConfigManager { config_dir, defaults, syntax_specific, overrides }
    }

    /// Generates a snapshot of the current configuration for `syntax`.
    pub fn get_config<S, V>(&self, syntax: S, buf_id: V) -> Config
        where S: Into<Option<SyntaxDefinition>>,
              V: Into<Option<BufferIdentifier>>
    {
        let syntax = syntax.into().unwrap_or_default();
        let buf_id = buf_id.into();
        let mut settings = match self.syntax_specific.get(&syntax) {
            Some(ref syntax_config) => self.defaults.merged_with(syntax_config),
            None => self.defaults.cache.clone(),
        };

        if let Some(overrides) = buf_id.and_then(|v| self.overrides.get(&v)) {
            merge_tables(&mut settings, &overrides.cache);
        }
        let settings: Value = settings.into();
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

    /// Sets a session-specific, buffer-specific override. The `from_user`
    /// flag indicates whether this override is coming via RPC (true) or
    /// from xi-core (false).
    pub fn set_override<K, V>(&mut self, key: K, value: V,
                              buf_id: BufferIdentifier, from_user: bool)
        where K: AsRef<str>,
              V: Into<Value>,
    {
        if !self.overrides.contains_key(&buf_id) {
            let conf_pair = ConfigPair::new(None, None);
            self.overrides.insert(buf_id.to_owned(), conf_pair);
        }
        self.overrides.get_mut(&buf_id)
            .unwrap()
            .set_override(key, value, from_user);
    }
}

impl Default for ConfigManager {
    fn default() -> ConfigManager {
        let config_dir = get_config_dir();
        let user_config_path = config_dir.join(XI_CONFIG_FILE_NAME);
        let user_config = load_config(&user_config_path).unwrap_or_default();
        let syntax_specific = load_syntax_configs(&config_dir);
        ConfigManager::new(&config_dir, user_config, syntax_specific)
    }
}

fn load_config(path: &Path) -> Result<Table, ()> {
    let conf: config::File<_> = path.into();
    conf.format(FileFormat::Toml)
        .collect()
        .map_err(|e| print_err!("Error reading config: {:?}", e))
}

/// Loads all of the syntax-specific config files in the target directory.
fn load_syntax_configs(config_dir: &Path) -> HashMap<SyntaxDefinition, Table> {
    let contents = config_dir.read_dir()
        .map(|dir| {
            dir.flat_map(Result::ok)
                .map(|p| p.path())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut result = HashMap::new();
    for config_path in contents {
        // config is invalid if path isn't utf-8; lossy gives better errors
        let file_name = config_path.file_name().unwrap().to_string_lossy();
        if !file_name.ends_with(".xiconfig") || file_name == XI_CONFIG_FILE_NAME {
            continue
        }

        let file_stem = config_path.file_stem().unwrap().to_string_lossy();
        let syntax = SyntaxDefinition::try_from_name(&file_stem);
        let conf = load_config(&config_path);
        match (syntax, conf) {
            (Some(s), Ok(c)) => { result.insert(s, c); }
            (None, _) => print_err!("unrecognized syntax name: {:?}",
                                           &file_stem),
            (_, Err(err)) => print_err!("Error parsing config {:?}\n{:?}",
                                        &config_path, err),
        }
    }
    result
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

/// Updates `base` with values in `other`.
fn merge_tables(base: &mut Table, other: &Table) {
    for (k, v) in other.iter() {
        base.insert(k.to_owned(), v.clone());
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
        let manager = ConfigManager::new("BASE_PATH", Table::default(),
                                         HashMap::new());
        let config = manager.get_config(None, None);
        assert_eq!(config.tab_size, 4);
        assert_eq!(config.plugin_search_path, vec![PathBuf::from("BASE_PATH/plugins")])
    }

    #[test]
    fn test_overrides() {
        let user_config = r#"tab_size = 42"#;
        let user_config = config::File::from_str(user_config, FileFormat::Toml)
            .collect()
            .unwrap();
        let rust_config = r#"tab_size = 31"#;
        let rust_config = config::File::from_str(rust_config, FileFormat::Toml)
            .collect()
            .unwrap();

        let mut user_syntax = HashMap::new();
        user_syntax.insert(SyntaxDefinition::Rust, rust_config);

        let mut manager = ConfigManager::new("", user_config, user_syntax);
        let buf_id = BufferIdentifier::new(1);
        manager.set_override("tab_size", 67, buf_id.clone(), false);

        let config = manager.get_config(None, None);
        assert_eq!(config.tab_size, 42);
        let config = manager.get_config(SyntaxDefinition::Yaml, None);
        assert_eq!(config.tab_size, 2);
        let config = manager.get_config(SyntaxDefinition::Yaml, buf_id.clone());
        assert_eq!(config.tab_size, 67);

        let config = manager.get_config(SyntaxDefinition::Rust, None);
        assert_eq!(config.tab_size, 31);
        let config = manager.get_config(SyntaxDefinition::Rust, buf_id.clone());
        assert_eq!(config.tab_size, 67);

        // user override trumps everything
        manager.set_override("tab_size", 85, buf_id.clone(), true);
        let config = manager.get_config(SyntaxDefinition::Rust, buf_id.clone());
        assert_eq!(config.tab_size, 85);
    }
}
