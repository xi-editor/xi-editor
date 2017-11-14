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
use std::io;
use std::borrow::Borrow;
use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::rc::Rc;
use std::path::{PathBuf, Path};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::de::Deserialize;
use config_rs::{self, Source, Value, FileFormat};

use syntax::SyntaxDefinition;
use tabs::BufferIdentifier;


static XI_CONFIG_DIR: &'static str = "XI_CONFIG_DIR";
static XDG_CONFIG_HOME: &'static str = "XDG_CONFIG_HOME";
/// A client can use this to pass a path to bundled plugins
static XI_SYS_PLUGIN_PATH: &'static str = "XI_SYS_PLUGIN_PATH";

/// Namespace for various default settings.
#[allow(unused)]
mod defaults {
    use super::*;
    pub const BASE: &'static str = include_str!("../assets/defaults.toml");
    pub const WINDOWS: &'static str = include_str!("../assets/windows.toml");
    pub const YAML: &'static str = include_str!("../assets/yaml.toml");
    pub const MAKEFILE: &'static str = include_str!("../assets/makefile.toml");

    /// config keys that are legal in most config files
    pub const GENERAL_KEYS: &'static [&'static str] = &[
        "tab_size",
        "newline",
        "translate_tabs_to_spaces",
    ];
    /// config keys that are only legal at the top level
    pub const TOP_LEVEL_KEYS: &'static [&'static str] = &[
        "plugin_search_path",
    ];

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
        config_rs::File::from_str(default, config_rs::FileFormat::Toml)
            .collect()
            .expect("default configs must load")
    }
}

/// A map of config keys to settings
pub type Table = HashMap<String, Value>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all="lowercase")]
/// A `ConfigDomain` describes a level or category of user settings.
pub enum ConfigDomain {
    /// The general user preferences
    Preferences,
    /// The overrides for a particular syntax.
    Syntax(SyntaxDefinition),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// The errors that can occur when managing configs.
pub enum ConfigError {
    /// The config contains a key that is invalid for its domain.
    IllegalKey(String),
    /// The config domain was not recognized.
    UnknownDomain(String),
    /// A file-based config could not be loaded or parsed.
    FileParse(PathBuf),
}

/// A `Validator` is responsible for validating a config table.
pub trait Validator: fmt::Debug {
    fn validate(&self, key: &str, value: &Value) -> Result<(), ConfigError>;
    fn validate_table(&self, table: &Table) -> Result<(), ConfigError> {
        for (key, value) in table.iter() {
            let _ = self.validate(key, value)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
/// An implementation of `Validator` that checks keys against a whitelist.
pub struct KeyValidator {
    keys: HashSet<String>,
}

/// Represents the common pattern of default settings masked by
/// user settings.
#[derive(Debug)]
pub struct ConfigPair {
    /// A static default configuration, which will never change.
    base: Option<Table>,
    /// A variable, user provided configuration. Items here take
    /// precedence over items in `base`.
    user: Option<Table>,
    /// A snapshot of base + user.
    cache: Arc<Table>,
    validator: Rc<Validator>,
}

#[derive(Debug)]
pub struct ConfigManager {
    /// The defaults, and any base user overrides
    defaults: ConfigPair,
    /// default per-syntax configs
    syntax_specific: HashMap<SyntaxDefinition, ConfigPair>,
    /// per-session overrides
    overrides: HashMap<BufferIdentifier, ConfigPair>,
    /// A map of paths to file based configs.
    sources: HashMap<PathBuf, ConfigDomain>,
    /// If using file-based config, this is the base config directory
    /// (perhaps `$HOME/.config/xi`, by default).
    config_dir: Option<PathBuf>,
    /// An optional client-provided path for bundled resources, such
    /// as plugins and themes.
    extras_dir: Option<PathBuf>,
}

/// A collection of config tables representing a heirarchy, with each
/// table's keys superceding keys in preceding tables.
#[derive(Debug, Clone, Default)]
struct TableStack(Vec<Arc<Table>>);

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A frozen collection of settings, and their sources.
pub struct Config<T> {
    /// The underlying set of config tables that contributed to this
    /// `Config` instance. Used for diffing.
    #[serde(skip)]
    source: TableStack,
    /// The settings themselves, deserialized into some concrete type.
    pub items: T,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
/// The concrete type for buffer-related settings.
pub struct BufferItems {
    pub newline: String,
    pub tab_size: usize,
    pub translate_tabs_to_spaces: bool,
}

pub type BufferConfig = Config<BufferItems>;

impl ConfigPair {
    fn new<T1, T2>(base: T1, user: T2, validator: Rc<Validator>)
                   -> Result<Self, ConfigError>
        where T1: Into<Option<Table>>,
              T2: Into<Option<Table>>,
    {
        let base = base.into();
        let user = user.into();
        let _ = user.as_ref()
            .map(|t| validator.validate_table(t))
            .unwrap_or(Ok(()))?;

        let cache = Arc::new(Table::new());
        let mut conf = ConfigPair { base, user, cache, validator };
        conf.rebuild();
        Ok(conf)
    }

    fn set_user(&mut self, user: Table) -> Result<(), ConfigError> {
        self.validator.validate_table(&user)?;
        self.user = Some(user);
        self.rebuild();
        Ok(())
    }

    fn rebuild(&mut self) {
        let mut cache = self.base.clone().unwrap_or_default();
        if let Some(ref user) = self.user {
            for (k, v) in user.iter() {
                cache.insert(k.to_owned(), v.clone());
            }
        }
        self.cache = Arc::new(cache);
    }

    /// Manually sets a key/value pair in one of `base` or `user`.
    ///
    /// Note: this is only intended to be used internally, when handling
    /// overrides.
    fn set_override<K, V>(&mut self, key: K, value: V, from_user: bool)
                          -> Result<(), ConfigError>
        where K: AsRef<str>,
              V: Into<Value>,
    {
        let key: String = key.as_ref().to_owned();
        let value = value.into();
        self.validator.validate(&key, &value)?;
        {
            let table = if from_user {
                self.user.get_or_insert(Table::new())
            } else {
                self.base.get_or_insert(Table::new())
            };
            table.insert(key, value);
        }
        self.rebuild();
        Ok(())
    }
}

impl ConfigManager {
    pub fn set_config_dir<P: AsRef<Path>>(&mut self, path: P) {
        self.config_dir = Some(path.as_ref().to_owned());
    }

    pub fn set_extras_dir<P: AsRef<Path>>(&mut self, path: P) {
        self.extras_dir = Some(path.as_ref().to_owned())
    }

    // NOTE: search paths don't really fit the general config model;
    // they're never exposed to the client, they can't be overridden on a
    // per-buffer basis, and they can be appended to from a number of sources.
    //
    // There is a reasonable argument that they should not be part of the
    // config system at all. For now, I'm treating them as a special case.
    /// Returns the plugin_search_path.
    pub fn plugin_search_path(&self) -> Vec<PathBuf> {
        let val = self.defaults.cache.get("plugin_search_path").unwrap();
        let mut search_path: Vec<PathBuf> = val.clone().try_into().unwrap();

        // relative paths should be relative to the config dir, if present
        if let Some(ref config_dir) = self.config_dir {
            search_path = search_path.iter()
                .map(|p| config_dir.join(p))
                .collect();
        }

        // append the client provided extras path, if present
        if let Some(ref sys_path) = self.extras_dir {
            search_path.push(sys_path.into());
        }
        search_path
    }

    /// Sets the config for the given domain, removing any existing config.
    pub fn update_config<P>(&mut self, domain: ConfigDomain, new_config: Table,
                            path: P) -> Result<(), ConfigError>
        where P: Into<Option<PathBuf>>,
    {
       let result = match domain {
            ConfigDomain::Preferences => self.defaults.set_user(new_config),
            ConfigDomain::Syntax(s) => self.set_user_syntax(s, new_config),
        };

       if result.is_ok() {
           if let Some(p) = path.into() {
               self.sources.insert(p, domain);
           }
       }
       result
    }

    /// If `path` points to a loaded config file, unloads the associated config.
    pub fn remove_source(&mut self, source: &Path) {
        if let Some(domain) = self.sources.remove(source) {
            self.update_config(domain, Table::new(), None)
                .expect("Empty table is always valid");
        }
    }

    /// Checks whether a given file should be loaded, i.e. whether it is a
    /// config file and whether it is in an expected location.
    pub fn should_load_file<P: AsRef<Path>>(&self, path: P) -> bool {
        let path = path.as_ref();

        path.extension() == Some(OsStr::new("xiconfig")) &&
            ConfigDomain::try_from_path(path).is_ok() &&
            self.config_dir.as_ref()
            .map(|p| Some(p.borrow()) == path.parent())
            .unwrap_or(false)
    }

    fn set_user_syntax(&mut self, syntax: SyntaxDefinition, config: Table)
                       -> Result<(), ConfigError> {
        let exists = self.syntax_specific.contains_key(&syntax);
        if exists {
            let syntax_pair = self.syntax_specific.get_mut(&syntax).unwrap();
            syntax_pair.set_user(config)
        } else {
            let syntax_pair = ConfigPair::new(None, config,
                                              KeyValidator::for_domain(syntax))?;
            self.syntax_specific.insert(syntax, syntax_pair);
            Ok(())
        }
    }

    /// Generates a snapshot of the current configuration for `syntax`.
    pub fn get_config<S, V>(&self, syntax: S, buf_id: V) -> BufferConfig
        where S: Into<Option<SyntaxDefinition>>,
              V: Into<Option<BufferIdentifier>>
    {
        let syntax = syntax.into().unwrap_or_default();
        let buf_id = buf_id.into();
        let mut configs = Vec::new();
        configs.push(self.defaults.cache.clone());

        if let Some(syntax_settings) = self.syntax_specific.get(&syntax) {
            configs.push(syntax_settings.cache.clone());
        }

        if let Some(overrides) = buf_id.and_then(|v| self.overrides.get(&v)) {
            configs.push(overrides.cache.clone());
        }

        let stack = TableStack(configs);
        stack.into_config()
    }

    /// Sets a session-specific, buffer-specific override. The `from_user`
    /// flag indicates whether this override is coming via RPC (true) or
    /// from xi-core (false).
    pub fn set_override<K, V>(&mut self, key: K, value: V,
                              buf_id: BufferIdentifier, from_user: bool)
                              -> Result<(), ConfigError>
        where K: AsRef<str>,
              V: Into<Value>,
    {
        if !self.overrides.contains_key(&buf_id) {
            let conf_pair = ConfigPair::new(
                None, None,
                KeyValidator::for_domain(SyntaxDefinition::default()))?;
            self.overrides.insert(buf_id.to_owned(), conf_pair);
        }
        self.overrides.get_mut(&buf_id)
            .unwrap()
            .set_override(key, value, from_user)
    }
}

impl Default for ConfigManager {
    fn default() -> ConfigManager {
        let defaults = ConfigPair::new(
            defaults::platform_defaults(), None,
            KeyValidator::for_domain(ConfigDomain::Preferences))
            .unwrap();
        let mut syntax_specific = defaults::syntax_defaults();
        let val = KeyValidator::for_domain(
            ConfigDomain::Syntax(SyntaxDefinition::default()));
        let syntax_specific = syntax_specific
            .drain()
            .map(|(k, v)| {
                (k.to_owned(), ConfigPair::new(v, None, val.clone()).unwrap())
            })
            .collect::<HashMap<_, _>>();

        // TODO: remove this when we finish migrating to client_init based setup
        let extras_dir = env::var(XI_SYS_PLUGIN_PATH).map(PathBuf::from).ok();

        ConfigManager {
            defaults: defaults,
            syntax_specific: syntax_specific,
            overrides: HashMap::new(),
            sources: HashMap::new(),
            config_dir: None,
            extras_dir: extras_dir,
        }
    }
}

impl TableStack {
    /// Create a single table representing the final config values.
    fn collate(&self) -> Table {
    // NOTE: This is fairly expensive; a future optimization would borrow
    // from the underlying collections, but then we couldn't take advantage of
    // config-rs's `try_into` for converting to a `Config`.
        let mut out = HashMap::new();
        for table in self.0.iter().rev() {
            for (k, v) in table.iter() {
                if !out.contains_key(k) {
                    // cloning these objects feels a bit gross, we could
                    // improve this by implementing Deserialize for TableStack.
                    out.insert(k.to_owned(), v.to_owned());
                }
            }
        }
        out
    }

    /// Converts the underlying tables into a static `Config` instance.
    fn into_config<'de, T: Deserialize<'de>>(self) -> Config<T> {
        let out = self.collate();
        let out: Value = out.into();
        let items: T = out.try_into().unwrap();
        let source = self;
        Config { source, items }
    }

    /// Walks the tables in priority (reverse) order, returning the first
    /// occurance of `key`.
    fn get<S: AsRef<str>>(&self, key: S) -> Option<&Value> {
        for table in self.0.iter().rev() {
            if let Some(v) = table.get(key.as_ref()) {
                return Some(v)
            }
        }
        None
    }

    /// Returns a new `Table` containing only those keys and values in `self`
    /// which have changed from `other`.
    fn diff(&self, other: &TableStack) -> Option<Table> {
        let mut out: Option<Table> = None;
        let this = self.collate();
        for (k, v) in this.iter() {
            if other.get(k) != Some(v) {
                let out: &mut Table = out.get_or_insert(Table::new());
                out.insert(k.to_owned(), v.to_owned());
            }
        }
        out
    }
}

impl<'de, T: Deserialize<'de>> Config<T> {
    /// Returns a `Table` of all the items in `self` which have different
    /// values than in `other`.
    pub fn changes_from_other(&self, other: &Config<T>) -> Option<Table> {
        self.source.diff(&other.source)
    }
}

impl<T: PartialEq> PartialEq for Config<T> {
    fn eq(&self, other: &Config<T>) -> bool {
        self.items == other.items
    }
}

impl ConfigDomain {
    /// Given a file path, attempts to parse the file name into a `ConfigDomain`.
    /// Returns an error if the file name does not correspond to a domain.
    pub fn try_from_path(path: &Path) -> Result<Self, ConfigError> {
        let file_stem = path.file_stem().unwrap().to_string_lossy();
        if file_stem == "preferences" {
            Ok(ConfigDomain::Preferences)
        } else if let Some(syntax) = SyntaxDefinition::try_from_name(&file_stem) {
            Ok(syntax.into())
        } else {
            Err(ConfigError::UnknownDomain(file_stem.into_owned()))
        }
    }
}

impl From<SyntaxDefinition> for ConfigDomain {
    fn from(src: SyntaxDefinition) -> ConfigDomain {
        ConfigDomain::Syntax(src)
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::ConfigError::*;
        match self {
            &IllegalKey(ref s) |
                &UnknownDomain(ref s) => write!(f, "{}: {}", self, s),
            &FileParse(ref p) => write!(f, "{}: {:?}", self, p),
        }
    }
}

impl Error for ConfigError {
    fn description(&self) -> &str {
        use self::ConfigError::*;
        match *self {
            IllegalKey( .. ) => "illegal key",
            UnknownDomain( .. ) => "unknown domain",
            FileParse( .. ) => "failed to parse file",
        }
    }
}

impl KeyValidator {
    /// Create a `KeyValidator` appropriate to the given domain.
    pub fn for_domain<D: Into<ConfigDomain>>(d: D) -> Rc<Self> {
        let keys = match d.into() {
            ConfigDomain::Preferences => defaults::GENERAL_KEYS.iter()
                .chain(defaults::TOP_LEVEL_KEYS.iter())
                .map(|s| String::from(*s))
                .collect(),
            ConfigDomain::Syntax(_) => defaults::GENERAL_KEYS.iter()
                .map(|s| String::from(*s))
                .collect(),
        };
        Rc::new(KeyValidator { keys })
    }
}

impl Validator for KeyValidator {
    fn validate(&self, key: &str, _value: &Value) -> Result<(), ConfigError>
    {
        if self.keys.contains(key) {
            Ok(())
        } else {
            Err(ConfigError::IllegalKey(key.to_owned()))
        }
    }
}

pub fn iter_config_files(dir: &Path) -> io::Result<Box<Iterator<Item=PathBuf>>> {
    let contents = dir.read_dir()?;
    let iter = contents.flat_map(Result::ok)
        .map(|p| p.path())
        .filter(|p| {
            p.extension().and_then(OsStr::to_str).unwrap_or("") == "xiconfig"
        });
    Ok(Box::new(iter))
}

/// Attempts to load a config from a file. The config's domain is determined
/// by the file name.
pub fn try_load_from_file(path: &Path) -> Result<(ConfigDomain, Table), Box<Error>> {
    let domain = ConfigDomain::try_from_path(path)?;
    let conf: config_rs::File<_> = path.into();
    let table = conf.format(FileFormat::Toml).collect()?;
    Ok((domain, table))
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

pub fn get_config_dir() -> PathBuf {
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
        let mut manager = ConfigManager::default();
        manager.set_config_dir("BASE_PATH");
        let config = manager.get_config(None, None);
        assert_eq!(config.items.tab_size, 4);
        assert_eq!(manager.plugin_search_path(), vec![PathBuf::from("BASE_PATH/plugins")])
    }

    #[test]
    fn test_overrides() {
        let user_config = r#"tab_size = 42"#;
        let user_config = config_rs::File::from_str(user_config, FileFormat::Toml)
            .collect()
            .unwrap();
        let rust_config = r#"tab_size = 31"#;
        let rust_config = config_rs::File::from_str(rust_config, FileFormat::Toml)
            .collect()
            .unwrap();

        let mut manager = ConfigManager::default();
        manager.update_config(ConfigDomain::Syntax(SyntaxDefinition::Rust),
                              rust_config, None).unwrap();

        manager.update_config(ConfigDomain::Preferences, user_config, None)
            .unwrap();

        let buf_id = BufferIdentifier::new(1);
        manager.set_override("tab_size", 67, buf_id.clone(), false).unwrap();

        let config = manager.get_config(None, None);
        assert_eq!(config.items.tab_size, 42);
        let config = manager.get_config(SyntaxDefinition::Yaml, None);
        assert_eq!(config.items.tab_size, 2);
        let config = manager.get_config(SyntaxDefinition::Yaml, buf_id.clone());
        assert_eq!(config.items.tab_size, 67);

        let config = manager.get_config(SyntaxDefinition::Rust, None);
        assert_eq!(config.items.tab_size, 31);
        let config = manager.get_config(SyntaxDefinition::Rust, buf_id.clone());
        assert_eq!(config.items.tab_size, 67);

        // user override trumps everything
        manager.set_override("tab_size", 85, buf_id.clone(), true).unwrap();
        let config = manager.get_config(SyntaxDefinition::Rust, buf_id.clone());
        assert_eq!(config.items.tab_size, 85);
    }

    #[test]
    fn test_validation() {
        let mut manager = ConfigManager::default();
        let user_config = r#"
tab_size = 42
font_frace = "InconsolableMo"
translate_tabs_to_spaces = true
"#;
        let user_config = config_from_toml_string(user_config);
        let r = manager.update_config(ConfigDomain::Preferences, user_config, None);
        assert_eq!(r, Err(ConfigError::IllegalKey("font_frace".into())));

        let syntax_config =  config_from_toml_string(r#"tab_size = 42
plugin_search_path = "/some/path"
translate_tabs_to_spaces = true"#);
        let r = manager.update_config(ConfigDomain::Syntax(SyntaxDefinition::Rust),
                                      syntax_config, None);
        // not valid in a syntax config
        assert_eq!(r, Err(ConfigError::IllegalKey("plugin_search_path".into())));
    }

    #[test]
    fn test_config_domain_serde() {
        assert!(ConfigDomain::try_from_path(Path::new("hi/python.xiconfig")).is_ok());
        assert!(ConfigDomain::try_from_path(Path::new("hi/preferences.xiconfig")).is_ok());
        assert!(ConfigDomain::try_from_path(Path::new("hi/rust.xiconfig")).is_ok());
        assert!(ConfigDomain::try_from_path(Path::new("hi/unknown.xiconfig")).is_err());
    }

    #[test]
    fn test_should_load() {
        let mut manager = ConfigManager::default();
        let config_dir = PathBuf::from("/home/config/xi");
        manager.set_config_dir(&config_dir);
        assert!(manager.should_load_file(&config_dir.join("preferences.xiconfig")));
        assert!(manager.should_load_file(&config_dir.join("rust.xiconfig")));
        assert!(!manager.should_load_file(&config_dir.join("fake?.xiconfig")));
        assert!(!manager.should_load_file(&config_dir.join("preferences.toml")));
        assert!(!manager.should_load_file(Path::new("/home/rust.xiconfig")));
        assert!(!manager.should_load_file(Path::new("/home/config/xi/subdir/rust.xiconfig")));
    }

    #[test]
    fn test_diff() {
        let conf1 = r#"
tab_size = 42
translate_tabs_to_spaces = true
"#;
        let conf1 = config_from_toml_string(conf1);

        let conf2 = r#"
tab_size = 6
translate_tabs_to_spaces = true
"#;
        let conf2 = config_from_toml_string(conf2);

        let stack1 = TableStack(vec![Arc::new(conf1)]);
        let stack2 = TableStack(vec![Arc::new(conf2)]);
        let diff = stack1.diff(&stack2).unwrap();
        assert!(diff.len() == 1);
        assert_eq!(diff.get("tab_size"), Some(&Value::new(None, 42)));
    }

    fn config_from_toml_string(toml: &str) -> Table {
        config_rs::File::from_str(toml, FileFormat::Toml)
            .collect()
            .unwrap()
    }
}
