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

use syntax::{LanguageId, Languages};
use tabs::{BufferId, ViewId};

/// Namespace for various default settings.
#[allow(unused)]
mod defaults {
    use super::*;
    pub const BASE: &str = include_str!("../assets/defaults.toml");
    pub const WINDOWS: &str = include_str!("../assets/windows.toml");

    /// A cache of loaded defaults.
    lazy_static! {
        static ref LOADED: Mutex<HashMap<ConfigDomain, Table>> = {
            Mutex::new(HashMap::new())
        };
    }

    /// Given a domain, returns the default config for that domain,
    /// if it exists.
    pub fn defaults_for_domain<D>(domain: D) -> Option<Table>
        where D: Into<ConfigDomain>,
    {
        let mut loaded = LOADED.lock().unwrap();
        let domain = domain.into();
        loaded.get(&domain).map(Table::to_owned)
    }

    pub fn insert<D>(domain: D, table: Table)
        where D: Into<ConfigDomain>,
    {
        let mut loaded = LOADED.lock().unwrap();
        loaded.insert(domain.into(), table);
    }

    /// Removes any default config present for `domain`.
    pub fn unload<D>(domain: D)
        where D: Into<ConfigDomain>,
    {
        let mut loaded = LOADED.lock().unwrap();
        loaded.remove(&domain.into());
    }

    pub fn load_base() {
        let mut base = load(BASE);
        if let Some(mut overrides) = platform_overrides() {
            for (k, v) in overrides.iter() {
                base.insert(k.to_owned(), v.to_owned());
            }
        }
        insert(ConfigDomain::General, base);
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all="snake_case")]
pub enum ConfigDomain {
    /// The general user preferences
    General,
    /// The overrides for a particular syntax.
    Language(LanguageId),
    /// The user overrides for a particular buffer
    UserOverride(BufferId),
    /// The system's overrides for a particular buffer. Only used internally.
    #[serde(skip_deserializing)]
    SysOverride(BufferId),
}

/// The external RPC sends `ViewId`s, which we convert to `BufferId`s
/// internally.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all="snake_case")]
pub enum ConfigDomainExternal {
    General,
    //TODO: remove this old name
    Syntax(LanguageId),
    Language(LanguageId),
    UserOverride(ViewId),
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
    languages: Languages,
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
    pub fn new(config_dir: Option<PathBuf>,
               extras_dir: Option<PathBuf>) -> Self
    {
        defaults::load_base();
        let mut defaults = HashMap::new();
        defaults.insert(ConfigDomain::General,
                        ConfigPair::for_domain(ConfigDomain::General));
        ConfigManager {
            configs: defaults,
            sources: HashMap::new(),
            languages: Languages::default(),
            config_dir,
            extras_dir,
        }
    }

    pub fn base_config_file_path(&self) -> Option<PathBuf> {
        let config_file = self.config_dir.as_ref()
            .map(|p| p.join("preferences.xiconfig"));
        let exists = config_file.as_ref().map(|p| p.exists())
            .unwrap_or(false);
        if exists { config_file } else { None }
    }

    pub fn get_plugin_paths(&self) -> Vec<PathBuf> {
        let config_dir = self.config_dir.as_ref().map(|p| p.join("plugins"));
        [self.extras_dir.as_ref(), config_dir.as_ref()].iter()
            .flat_map(|p| p.map(|p| p.to_owned()))
            .filter(|p| p.exists())
            .collect()
    }

    /// Set the available `LanguageDefinition`s. Overrides any previous values.
    pub fn set_languages(&mut self, languages: Languages) {
        // if any languages have been removed, remove their default settings
        // we arguably don't need to do this, since with the language removed
        // the settings for that language should be inaccessible? But this
        // feels honest.
        self.languages.difference(&languages)
            .iter()
            .for_each(|lang| defaults::unload(lang.name.clone()));

        for language in languages.iter() {
            let lang_id = language.name.clone();
            if let Some(ref config) = language.default_config {
                eprintln!("loaded config for {:?}: {:?}", &lang_id, config);
                defaults::insert(lang_id.clone(), config.clone())
            } else {
                // if a lang still exists but has lost its default config?
                defaults::unload(lang_id.clone());
            }

            let domain: ConfigDomain = lang_id.clone().into();
            self.configs.entry(domain.clone())
                .and_modify(|pair| {
                    pair.base = defaults::defaults_for_domain(lang_id.clone());
                    pair.rebuild();
                })
                .or_insert_with(|| ConfigPair::for_domain(domain));
        }

        self.languages = languages;
    }

    pub fn language_for_path(&self, path: &Path) -> Option<LanguageId> {
        self.languages.language_for_path(path)
            .map(|lang| lang.name.clone())
    }

    /// Sets the config for the given domain, removing any existing config.
    pub fn set_user_config<P>(&mut self, domain: ConfigDomain,
                              new_config: Table, path: P)
                              -> Result<(), ConfigError>
        where P: Into<Option<PathBuf>>,
    {
        self.check_table(&new_config)?;
        self.configs.entry(domain.clone())
            .or_insert_with(|| { ConfigPair::for_domain(domain.clone()) })
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
        let conf = self.configs.entry(domain.clone())
            .or_insert_with(|| { ConfigPair::for_domain(domain) });
        conf.update_table(changes);
        Ok(())
    }

    pub fn domain_for_path(&self, path: &Path) -> Option<ConfigDomain> {
        if path.extension().map(|e| e != "xiconfig").unwrap_or(true) {
            return None;
        }
        match path.file_stem().and_then(|s| s.to_str()) {
            Some("preferences") => Some(ConfigDomain::General),
            Some(name) if self.languages.language_for_name(&name).is_some() => {
                let lang = self.languages.language_for_name(&name)
                    .map(|lang| lang.name.clone()).unwrap();
                Some(ConfigDomain::Language(lang))
            }
            //TODO: plugin configs
            _ => None,
        }
    }

    /// If `path` points to a loaded config file, unloads the associated config.
    pub fn remove_source(&mut self, source: &Path) {
        if let Some(domain) = self.sources.remove(source) {
            self.set_user_config(domain, Table::new(), None)
                .expect("Empty table is always valid");
        }
    }

    //TODO: remove this whole fn
    /// Checks whether a given file should be loaded, i.e. whether it is a
    /// config file and whether it is in an expected location.
    pub fn should_load_file<P: AsRef<Path>>(&self, path: P) -> bool {
        path.as_ref().extension() == Some(OsStr::new("xiconfig"))
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
    pub fn get_buffer_config<S, I>(&self, lang: S, id: I) -> BufferConfig
        where S: Into<Option<LanguageId>>,
              I: Into<Option<BufferId>>
    {
        let lang = lang.into();
        let id = id.into();
        let mut configs = Vec::new();

        configs.push(self.configs.get(&ConfigDomain::General));
        lang.map(|s| configs.push(self.configs.get(&s.into())));
        id.map(|v| configs.push(self.configs.get(&ConfigDomain::SysOverride(v))));
        id.map(|v| configs.push(self.configs.get(&ConfigDomain::UserOverride(v))));

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
}

impl TableStack {
    /// Create a single table representing the final config values.
    fn collate(&self) -> Table {
    // NOTE: This is fairly expensive; a future optimization would borrow
    // from the underlying collections.
        let mut out = Table::new();
        for table in &self.0 {
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
        for table in &self.0 {
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

impl From<LanguageId> for ConfigDomain {
    fn from(src: LanguageId) -> ConfigDomain {
        ConfigDomain::Language(src)
    }
}

impl From<BufferId> for ConfigDomain {
    fn from(src: BufferId) -> ConfigDomain {
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
    builder.create(dir.join("plugins"))?;
    Ok(())
}

/// Attempts to load a config from a file. The config's domain is determined
/// by the file name.
pub fn try_load_from_file(path: &Path) -> Result<Table, ConfigError> {
    let mut file = fs::File::open(&path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    table_from_toml_str(&contents)
        .map_err(|e| ConfigError::Parse(path.to_owned(), e))
}

pub(crate) fn table_from_toml_str(s: &str) -> Result<Table, toml::de::Error> {
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
    fn test_overrides() {
        let user_config = table_from_toml_str(r#"tab_size = 42"#).unwrap();
        let rust_config = table_from_toml_str(r#"tab_size = 31"#).unwrap();

        let rust_id: LanguageId = "Rust".into();

        let mut manager = ConfigManager::new(None, None);
        manager.set_user_config(ConfigDomain::Language(rust_id.clone()),
                                rust_config, None).unwrap();

        manager.set_user_config(ConfigDomain::General, user_config, None)
            .unwrap();

        let buffer_id = BufferId(1);
        // system override
        let changes = json!({"tab_size": 67}).as_object().unwrap().to_owned();
        manager.update_user_config(ConfigDomain::SysOverride(buffer_id), changes)
            .unwrap();

        let config = manager.default_buffer_config();
        assert_eq!(config.source.0.len(), 1);
        assert_eq!(config.items.tab_size, 42);
        let config = manager.get_buffer_config(rust_id.clone(), None);
        assert_eq!(config.items.tab_size, 31);
        let config = manager.get_buffer_config(rust_id.clone(), buffer_id);
        assert_eq!(config.items.tab_size, 67);

        // user override trumps everything
        let changes = json!({"tab_size": 85}).as_object().unwrap().to_owned();
        manager.update_user_config(ConfigDomain::UserOverride(buffer_id), changes)
            .unwrap();
        let config = manager.get_buffer_config(rust_id.clone(), buffer_id);
        assert_eq!(config.items.tab_size, 85);
    }

    #[test]
    fn test_config_domain_serde() {
        assert_eq!(serde_json::to_string(&ConfigDomain::General).unwrap(), "\"general\"");
        let d = ConfigDomainExternal::UserOverride(ViewId(1));
        assert_eq!(serde_json::to_string(&d).unwrap(), "{\"user_override\":\"view-id-1\"}");
        let d = ConfigDomain::Language("Swift".into());
        assert_eq!(serde_json::to_string(&d).unwrap(), "{\"language\":\"Swift\"}");
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
        let mut manager = ConfigManager::new(None, None);
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
        manager.update_user_config(LanguageId::from("Dart").into(), changes).unwrap();
        let config = manager.get_buffer_config(LanguageId::from("Dart"), None);
        assert_eq!(config.items.font_face, "Roboto");
    }
}
