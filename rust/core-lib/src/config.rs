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

use std::io::{self, Read};
use std::borrow::Borrow;
use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{PathBuf, Path};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::de::Deserialize;
use serde_json::{self, Value};
use toml;

use syntax::SyntaxDefinition;
use tabs::ViewIdentifier;

/// Namespace for various default settings.
#[allow(unused)]
mod defaults {
    use super::*;
    pub const BASE: &str = include_str!("../assets/defaults.toml");
    pub const WINDOWS: &str = include_str!("../assets/windows.toml");
    pub const YAML: &str = include_str!("../assets/yaml.toml");
    pub const MAKEFILE: &str = include_str!("../assets/makefile.toml");

    /// A cache of loaded defaults.
    lazy_static! {
        static ref LOADED: Mutex<HashMap<ConfigDomain, Option<Table>>> = {
            Mutex::new(HashMap::new())
        };
    }


    /// config keys that are legal in most config files
    pub const GENERAL_KEYS: &[&str] = &[
        "tab_size",
        "line_ending",
        "translate_tabs_to_spaces",
        "use_tab_stops",
        "font_face",
        "font_size",
        "auto_indent",
        "scroll_past_end",
        "wrap_width",
    ];
    /// config keys that are only legal at the top level
    pub const TOP_LEVEL_KEYS: &[&str] = &[
        "plugin_search_path",
    ];

    /// Given a domain, returns the default config for that domain,
    /// if it exists.
    pub fn defaults_for_domain<D>(domain: D) -> Option<Table>
        where D: Into<ConfigDomain>,
    {
        let mut loaded = LOADED.lock().unwrap();
        let domain = domain.into();
        loaded.entry(domain).or_insert_with(|| { load_for_domain(domain) })
            .to_owned()
    }

    fn load_for_domain(domain: ConfigDomain) -> Option<Table> {
        match domain.into() {
            ConfigDomain::General => {
                let mut base = load(BASE);
                if let Some(mut overrides) = platform_overrides() {
                    for (k, v) in overrides.iter() {
                        base.insert(k.to_owned(), v.to_owned());
                    }
                }
                Some(base)
            }
            ConfigDomain::Syntax(SyntaxDefinition::Yaml) =>
                Some(load(YAML)),
            ConfigDomain::Syntax(SyntaxDefinition::Makefile) =>
                Some(load(MAKEFILE)),
            _ => None,
        }
    }

    fn platform_overrides() -> Option<Table> {
        #[cfg(target_os = "windows")]
        { return Some(load(WINDOWS)) }
        None
    }

    fn load(default: &str) -> Table {
        table_from_toml_str(default)
            .expect("default configs must load")
    }
}

/// A map of config keys to settings
pub type Table = serde_json::Map<String, Value>;

/// A `ConfigDomain` describes a level or category of user settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all="snake_case")]
pub enum ConfigDomain {
    /// The general user preferences
    General,
    /// The overrides for a particular syntax.
    Syntax(SyntaxDefinition),
    /// The user overrides for a particular buffer
    UserOverride(ViewIdentifier),
    /// The system's overrides for a particular buffer. Only used internally.
    #[serde(skip_deserializing)]
    SysOverride(ViewIdentifier),
}

/// The errors that can occur when managing configs.
#[derive(Debug)]
pub enum ConfigError {
    /// The config domain was not recognized.
    UnknownDomain(String),
    /// A file-based config could not be loaded or parsed.
    Parse(PathBuf, toml::de::Error),
    /// The config table contained unexpected values
    UnexpectedItem(serde_json::Error),
    /// An Io Error
    Io(io::Error),
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
}

#[derive(Debug)]
pub struct ConfigManager {
    /// A map of `ConfigPairs` (defaults + overrides) for all in-use domains.
    configs: HashMap<ConfigDomain, ConfigPair>,
    /// A map of paths to file based configs.
    sources: HashMap<PathBuf, ConfigDomain>,
    /// If using file-based config, this is the base config directory
    /// (perhaps `$HOME/.config/xi`, by default).
    config_dir: Option<PathBuf>,
    /// An optional client-provided path for bundled resources, such
    /// as plugins and themes.
    extras_dir: Option<PathBuf>,
}

/// A collection of config tables representing a hierarchy, with each
/// table's keys superseding keys in preceding tables.
#[derive(Debug, Clone, Default)]
struct TableStack(Vec<Arc<Table>>);

/// A frozen collection of settings, and their sources.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config<T> {
    /// The underlying set of config tables that contributed to this
    /// `Config` instance. Used for diffing.
    #[serde(skip)]
    source: TableStack,
    /// The settings themselves, deserialized into some concrete type.
    pub items: T,
}

/// The concrete type for buffer-related settings.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct BufferItems {
    pub line_ending: String,
    pub tab_size: usize,
    pub translate_tabs_to_spaces: bool,
    pub use_tab_stops: bool,
    pub font_face: String,
    pub font_size: f32,
    pub auto_indent: bool,
    pub scroll_past_end: bool,
    pub wrap_width: usize,
}

pub type BufferConfig = Config<BufferItems>;

impl ConfigPair {
    /// Creates a new `ConfigPair` suitable for the provided domain.
    fn for_domain<D: Into<ConfigDomain>>(domain: D) -> Self {
        let domain = domain.into();
        let base = defaults::defaults_for_domain(domain);
        let user = None;
        let cache = Arc::new(base.clone().unwrap_or_default());
        ConfigPair { base, user, cache }
    }

    fn set_table(&mut self, user: Table) {
        self.user = Some(user);
        self.rebuild();
    }

    fn update_table(&mut self, changes: Table) {
        {
            let conf = self.user.get_or_insert(Table::new());
            for (k, v) in changes {
                //TODO: test/document passing null values to unset keys
                if v.is_null() {
                    conf.remove(&k);
                } else {
                    conf.insert(k.to_owned(), v.to_owned());
                }
            }
        }
        self.rebuild();
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
        let val = self.get("plugin_search_path", ConfigDomain::General).unwrap();
        let mut search_path: Vec<PathBuf> = serde_json::from_value(val.clone())
            .unwrap();

        // if there is user config dir, add plugins subdir to search path
        if let Some(ref config_dir) = self.config_dir {
            search_path.push(config_dir.join("plugins"));
        }

        // append the client provided extras path, if present
        if let Some(ref sys_path) = self.extras_dir {
            search_path.push(sys_path.into());
        }
        search_path
    }

    /// Sets the config for the given domain, removing any existing config.
    pub fn set_user_config<P>(&mut self, domain: ConfigDomain,
                              new_config: Table, path: P)
                              -> Result<(), ConfigError>
        where P: Into<Option<PathBuf>>,
    {
        self.check_table(&new_config)?;
        self.configs.entry(domain.into())
            .or_insert_with(|| { ConfigPair::for_domain(domain) })
            .set_table(new_config);
        path.into().map(|p| self.sources.insert(p, domain));
        Ok(())
    }

    /// Updates the config for the given domain. Existing keys which are
    /// not in `changes` are untouched; existing keys for which `changes`
    /// contains `Value::Null` are removed.
    pub fn update_user_config(&mut self, domain: ConfigDomain, changes: Table)
                          -> Result<(), ConfigError>
    {
        self.check_table(&changes)?;
        let conf = self.configs.entry(domain.into())
            .or_insert_with(|| { ConfigPair::for_domain(domain) });
        conf.update_table(changes);
        Ok(())
    }

    /// If `path` points to a loaded config file, unloads the associated config.
    pub fn remove_source(&mut self, source: &Path) {
        if let Some(domain) = self.sources.remove(source) {
            self.set_user_config(domain, Table::new(), None)
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

    fn check_table(&self, table: &Table) -> Result<(), ConfigError> {
        // verify that this table is well formed
        let mut defaults = defaults::defaults_for_domain(ConfigDomain::General)
            .expect("general domain must have defaults");
        for (k, v) in table.iter() {
            // changes can include 'null', which means clear field
            if v.is_null() { continue }
            defaults.insert(k.to_owned(), v.to_owned());
        }
        let _: BufferItems = serde_json::from_value(defaults.into())?;
        Ok(())
    }

    /// Generates a snapshot of the current configuration for a particular
    /// view.
    pub fn get_buffer_config<S, V>(&self, syntax: S, view_id: V) -> BufferConfig
        where S: Into<Option<SyntaxDefinition>>,
              V: Into<Option<ViewIdentifier>>
    {
        let syntax = syntax.into();
        let view_id = view_id.into();
        let mut configs = Vec::new();

        configs.push(self.configs.get(&ConfigDomain::General));
        syntax.map(|s| configs.push(self.configs.get(&s.into())));
        view_id.map(|v| configs.push(self.configs.get(&ConfigDomain::SysOverride(v))));
        view_id.map(|v| configs.push(self.configs.get(&ConfigDomain::UserOverride(v))));

        let configs = configs.iter().flat_map(Option::iter)
            .map(|c| c.cache.clone())
            .rev()
            .collect::<Vec<_>>();

        let stack = TableStack(configs);
        stack.into_config()
    }

    pub fn default_buffer_config(&self) -> BufferConfig {
        self.get_buffer_config(None, None)
    }

    /// Return the value for `key` in the `ConfigDomain` `domain`.
    fn get<D>(&self, key: &str, domain: D) -> Option<&Value>
        where D: Into<ConfigDomain>,
    {
        self.configs.get(&domain.into())
            .and_then(|c| c.cache.get(key))
    }
}

impl Default for ConfigManager {
    fn default() -> ConfigManager {
        // the domains for which we include defaults (platform defaults are
        // rolled into `General` at runtime)
        let defaults = vec![
            ConfigDomain::General,
            ConfigDomain::Syntax(SyntaxDefinition::Yaml),
            ConfigDomain::Syntax(SyntaxDefinition::Makefile)
        ].iter()
        .map(|d| (*d, ConfigPair::for_domain(*d)))
        .collect::<HashMap<_, _>>();

        ConfigManager {
            configs: defaults,
            sources: HashMap::new(),
            config_dir: None,
            extras_dir: None,
        }
    }
}

impl TableStack {
    /// Create a single table representing the final config values.
    fn collate(&self) -> Table {
    // NOTE: This is fairly expensive; a future optimization would borrow
    // from the underlying collections.
        let mut out = Table::new();
        for table in self.0.iter() {
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
    fn into_config<T>(self) -> Config<T>
        where for<'de> T: Deserialize<'de>
    {
        let out = self.collate();
        let items: T = serde_json::from_value(out.into()).unwrap();
        let source = self;
        Config { source, items }
    }

    /// Walks the tables in priority order, returning the first
    /// occurance of `key`.
    fn get<S: AsRef<str>>(&self, key: S) -> Option<&Value> {
        for table in self.0.iter() {
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

impl<T> Config<T> {
    pub fn to_table(&self) -> Table {
        self.source.collate()
    }
}

impl<'de, T: Deserialize<'de>> Config<T> {
    /// Returns a `Table` of all the items in `self` which have different
    /// values than in `other`.
    pub fn changes_from(&self, other: Option<&Config<T>>) -> Option<Table> {
        match other {
            Some(other) => self.source.diff(&other.source),
            None => self.source.collate().into(),
        }
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
            Ok(ConfigDomain::General)
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

impl From<ViewIdentifier> for ConfigDomain {
    fn from(src: ViewIdentifier) -> ConfigDomain {
        ConfigDomain::UserOverride(src)
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::ConfigError::*;
        match *self {
            UnknownDomain(ref s) => write!(f, "{}: {}", self.description(), s),
            Parse(ref p, ref e) => write!(f, "{} ({:?}), {:?}", self.description(), p, e),
            Io(ref e) => write!(f, "error loading config: {:?}", e),
            UnexpectedItem( ref e ) => write!(f, "{}", e),
        }
    }
}

impl Error for ConfigError {
    fn description(&self) -> &str {
        use self::ConfigError::*;
        match *self {
            UnknownDomain( .. ) => "unknown domain",
            Parse( _, ref e ) => e.description(),
            Io( ref e ) => e.description(),
            UnexpectedItem( ref e ) => e.description(),
        }
    }
}

impl From<io::Error> for ConfigError {
    fn from(src: io::Error) -> ConfigError {
        ConfigError::Io(src)
    }
}

impl From<serde_json::Error> for ConfigError {
    fn from(src: serde_json::Error) -> ConfigError {
        ConfigError::UnexpectedItem(src)
    }
}

/// Creates initial config directory structure
pub fn init_config_dir(dir: &Path) -> io::Result<()> {
    let builder = fs::DirBuilder::new();
    builder.create(dir)?;
    Ok(builder.create(dir.join("plugins"))?)
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
pub fn try_load_from_file(path: &Path) -> Result<(ConfigDomain, Table), ConfigError> {
    let domain = ConfigDomain::try_from_path(path)?;
    let mut file = fs::File::open(&path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let table = table_from_toml_str(&contents)
        .map_err(|e| ConfigError::Parse(path.to_owned(), e))?;

    Ok((domain, table))
}

fn table_from_toml_str(s: &str) -> Result<Table, toml::de::Error> {
    let table = toml::from_str(&s)?;
    let table = from_toml_value(table).as_object()
        .unwrap()
        .to_owned();
    Ok(table)
}

//adapted from https://docs.rs/crate/config/0.7.0/source/src/file/format/toml.rs
/// Converts between toml (used to write config files) and json
/// (used to store config values internally).
fn from_toml_value(value: toml::Value) -> Value {
    match value {
        toml::Value::String(value) => value.to_owned().into(),
        toml::Value::Float(value) => value.into(),
        toml::Value::Integer(value) => value.into(),
        toml::Value::Boolean(value) => value.into(),
        toml::Value::Datetime(value) => value.to_string().into(),

        toml::Value::Table(table) => {
            let mut m = Table::new();
            for (key, value) in table {
                m.insert(key.clone(), from_toml_value(value));
            }
            m.into()
        }

        toml::Value::Array(array) => {
            let mut l = Vec::new();
            for value in array {
                l.push(from_toml_value(value));
            }
            l.into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prepend_path() {
        let mut manager = ConfigManager::default();
        manager.set_config_dir("BASE_PATH");
        let config = manager.default_buffer_config();
        assert_eq!(config.items.tab_size, 4);
        assert_eq!(manager.plugin_search_path(), vec![PathBuf::from("BASE_PATH/plugins")])
    }

    #[test]
    fn test_loading_defaults() {
        let manager = ConfigManager::default();
        assert_eq!(manager.configs.len(), 3);
        let key = SyntaxDefinition::Yaml.into();
        assert!(manager.configs.contains_key(&key));
        let yaml = manager.configs.get(&key).unwrap();
        assert_eq!(yaml.cache.get("tab_size"), Some(&json!(2)));
    }

    #[test]
    fn test_overrides() {
        let user_config = table_from_toml_str(r#"tab_size = 42"#).unwrap();
        let rust_config = table_from_toml_str(r#"tab_size = 31"#).unwrap();

        let mut manager = ConfigManager::default();
        manager.set_user_config(ConfigDomain::Syntax(SyntaxDefinition::Rust),
                                rust_config, None).unwrap();

        manager.set_user_config(ConfigDomain::General, user_config, None)
            .unwrap();

        let view_id = "view-id-1".into();
        // system override
        let changes = json!({"tab_size": 67}).as_object().unwrap().to_owned();
        manager.update_user_config(ConfigDomain::SysOverride(view_id), changes).unwrap();

        let config = manager.default_buffer_config();
        assert_eq!(config.source.0.len(), 1);
        assert_eq!(config.items.tab_size, 42);
        // yaml defaults set this to 2
        let config = manager.get_buffer_config(SyntaxDefinition::Yaml, None);
        assert_eq!(config.source.0.len(), 2);
        assert_eq!(config.items.tab_size, 2);
        let config = manager.get_buffer_config(SyntaxDefinition::Yaml, view_id);
        assert_eq!(config.source.0.len(), 3);
        assert_eq!(config.items.tab_size, 67);

        let config = manager.get_buffer_config(SyntaxDefinition::Rust, None);
        assert_eq!(config.items.tab_size, 31);
        let config = manager.get_buffer_config(SyntaxDefinition::Rust, view_id);
        assert_eq!(config.items.tab_size, 67);

        // user override trumps everything
        let changes = json!({"tab_size": 85}).as_object().unwrap().to_owned();
        manager.update_user_config(ConfigDomain::UserOverride(view_id), changes).unwrap();
        let config = manager.get_buffer_config(SyntaxDefinition::Rust, view_id);
        assert_eq!(config.items.tab_size, 85);
    }

    #[test]
    fn test_config_domain_serde() {
        assert!(ConfigDomain::try_from_path(Path::new("hi/python.xiconfig")).is_ok());
        assert!(ConfigDomain::try_from_path(Path::new("hi/preferences.xiconfig")).is_ok());
        assert!(ConfigDomain::try_from_path(Path::new("hi/rust.xiconfig")).is_ok());
        assert!(ConfigDomain::try_from_path(Path::new("hi/unknown.xiconfig")).is_err());

        assert_eq!(serde_json::to_string(&ConfigDomain::General).unwrap(), "\"general\"");
        let d = ConfigDomain::UserOverride(ViewIdentifier::from("view-id-1"));
        assert_eq!(serde_json::to_string(&d).unwrap(), "{\"user_override\":\"view-id-1\"}");
        let d = ConfigDomain::Syntax(SyntaxDefinition::Swift);
        assert_eq!(serde_json::to_string(&d).unwrap(), "{\"syntax\":\"swift\"}");
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
        let conf1 = table_from_toml_str(conf1).unwrap();

        let conf2 = r#"
tab_size = 6
translate_tabs_to_spaces = true
"#;
        let conf2 = table_from_toml_str(conf2).unwrap();

        let stack1 = TableStack(vec![Arc::new(conf1)]);
        let stack2 = TableStack(vec![Arc::new(conf2)]);
        let diff = stack1.diff(&stack2).unwrap();
        assert!(diff.len() == 1);
        assert_eq!(diff.get("tab_size"), Some(&42.into()));
    }

    #[test]
    fn test_updating_in_place() {
        let mut manager = ConfigManager::default();
        assert_eq!(manager.default_buffer_config().items.font_size, 14.);
        let changes = json!({"font_size": 69, "font_face": "nice"})
            .as_object().unwrap().to_owned();
        manager.update_user_config(ConfigDomain::General, changes).unwrap();
        assert_eq!(manager.default_buffer_config().items.font_size, 69.);

        // null values in updates removes keys
        let changes = json!({"font_size": Value::Null})
            .as_object().unwrap().to_owned();
        manager.update_user_config(ConfigDomain::General, changes).unwrap();
        assert_eq!(manager.default_buffer_config().items.font_size, 14.);
        assert_eq!(manager.default_buffer_config().items.font_face, "nice");

        let changes = json!({"font_face": "Roboto"})
            .as_object().unwrap().to_owned();
        manager.update_user_config(SyntaxDefinition::Dart.into(), changes).unwrap();
        let config = manager.get_buffer_config(SyntaxDefinition::Dart, None);
        assert_eq!(config.items.font_face, "Roboto");
    }
}
