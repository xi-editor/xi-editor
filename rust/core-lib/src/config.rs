// Copyright 2017 The xi-editor Authors.
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

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::de::{self, Deserialize};
use serde_json::{self, Value};

use crate::syntax::{LanguageId, Languages};
use crate::tabs::{BufferId, ViewId};

/// Loads the included base config settings.
fn load_base_config() -> Table {
    fn load(default: &str) -> Table {
        table_from_toml_str(default).expect("default configs must load")
    }

    fn platform_overrides() -> Option<Table> {
        if cfg!(test) {
            // Exit early if we are in tests and never have platform overrides.
            // This makes sure we have a stable test environment.
            None
        } else if cfg!(windows) {
            let toml = include_str!("../assets/windows.toml");
            Some(load(toml))
        } else {
            // All other platorms
            None
        }
    }

    let base_toml: &str = include_str!("../assets/defaults.toml");
    let mut base = load(base_toml);
    if let Some(overrides) = platform_overrides() {
        for (k, v) in overrides.iter() {
            base.insert(k.to_owned(), v.to_owned());
        }
    }
    base
}

/// A map of config keys to settings
pub type Table = serde_json::Map<String, Value>;

/// A `ConfigDomain` describes a level or category of user settings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
#[serde(rename_all = "snake_case")]
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
    base: Option<Arc<Table>>,
    /// A variable, user provided configuration. Items here take
    /// precedence over items in `base`.
    user: Option<Arc<Table>>,
    /// A snapshot of base + user.
    cache: Arc<Table>,
}

/// The language associated with a given buffer; this is always detected
/// but can also be manually set by the user.
#[derive(Debug, Clone)]
struct LanguageTag {
    detected: LanguageId,
    user: Option<LanguageId>,
}

#[derive(Debug)]
pub struct ConfigManager {
    /// A map of `ConfigPairs` (defaults + overrides) for all in-use domains.
    configs: HashMap<ConfigDomain, ConfigPair>,
    /// The currently loaded `Languages`.
    languages: Languages,
    /// The language assigned to each buffer.
    buffer_tags: HashMap<BufferId, LanguageTag>,
    /// The configs for any open buffers
    buffer_configs: HashMap<BufferId, BufferConfig>,
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

fn deserialize_tab_size<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let tab_size = usize::deserialize(deserializer)?;
    if tab_size == 0 {
        Err(de::Error::invalid_value(
            de::Unexpected::Unsigned(tab_size as u64),
            &"tab_size must be at least 1",
        ))
    } else {
        Ok(tab_size)
    }
}

/// The concrete type for buffer-related settings.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct BufferItems {
    pub line_ending: String,
    #[serde(deserialize_with = "deserialize_tab_size")]
    pub tab_size: usize,
    pub translate_tabs_to_spaces: bool,
    pub use_tab_stops: bool,
    pub font_face: String,
    pub font_size: f32,
    pub auto_indent: bool,
    pub scroll_past_end: bool,
    pub wrap_width: usize,
    pub word_wrap: bool,
    pub autodetect_whitespace: bool,
    pub surrounding_pairs: Vec<(String, String)>,
    pub save_with_newline: bool,
}

pub type BufferConfig = Config<BufferItems>;

impl ConfigPair {
    /// Creates a new `ConfigPair` with the provided base config.
    fn with_base<T: Into<Option<Table>>>(table: T) -> Self {
        let base = table.into().map(Arc::new);
        let cache = base.clone().unwrap_or_default();
        ConfigPair { base, cache, user: None }
    }

    /// Returns a new `ConfigPair` with the provided base and the current
    /// user config.
    fn new_with_base<T: Into<Option<Table>>>(&self, table: T) -> Self {
        let mut new_self = ConfigPair::with_base(table);
        new_self.user = self.user.clone();
        new_self.rebuild();
        new_self
    }

    fn set_table(&mut self, user: Table) {
        self.user = Some(Arc::new(user));
        self.rebuild();
    }

    /// Returns the `Table` produced by updating `self.user` with the contents
    /// of `user`, deleting null entries.
    fn table_for_update(&self, user: Table) -> Table {
        let mut new_user: Table =
            self.user.as_ref().map(|arc| arc.as_ref().clone()).unwrap_or_default();
        for (k, v) in user {
            if v.is_null() {
                new_user.remove(&k);
            } else {
                new_user.insert(k, v);
            }
        }
        new_user
    }

    fn rebuild(&mut self) {
        let mut cache = self.base.clone().unwrap_or_default();
        if let Some(ref user) = self.user {
            for (k, v) in user.iter() {
                Arc::make_mut(&mut cache).insert(k.to_owned(), v.clone());
            }
        }
        self.cache = cache;
    }
}

impl ConfigManager {
    pub fn new(config_dir: Option<PathBuf>, extras_dir: Option<PathBuf>) -> Self {
        let base = load_base_config();
        let mut defaults = HashMap::new();
        defaults.insert(ConfigDomain::General, ConfigPair::with_base(base));
        ConfigManager {
            configs: defaults,
            buffer_tags: HashMap::new(),
            buffer_configs: HashMap::new(),
            languages: Languages::default(),
            config_dir,
            extras_dir,
        }
    }

    /// The path of the user's config file, if present.
    pub(crate) fn base_config_file_path(&self) -> Option<PathBuf> {
        let config_file = self.config_dir.as_ref().map(|p| p.join("preferences.xiconfig"));
        let exists = config_file.as_ref().map(|p| p.exists()).unwrap_or(false);
        if exists {
            config_file
        } else {
            None
        }
    }

    pub(crate) fn get_plugin_paths(&self) -> Vec<PathBuf> {
        let config_dir = self.config_dir.as_ref().map(|p| p.join("plugins"));
        [self.extras_dir.as_ref(), config_dir.as_ref()]
            .iter()
            .flat_map(|p| p.map(|p| p.to_owned()))
            .filter(|p| p.exists())
            .collect()
    }

    /// Adds a new buffer to the config manager, and returns the initial config
    /// `Table` for that buffer. The `path` argument is used to determine
    /// the buffer's default language.
    ///
    /// # Note: The caller is responsible for ensuring the config manager is
    /// notified every time a buffer is added or removed.
    ///
    /// # Panics:
    ///
    /// Panics if `id` already exists.
    pub(crate) fn add_buffer(&mut self, id: BufferId, path: Option<&Path>) -> Table {
        let lang =
            path.and_then(|p| self.language_for_path(p)).unwrap_or(LanguageId::from("Plain Text"));
        let lang_tag = LanguageTag::new(lang);
        assert!(self.buffer_tags.insert(id, lang_tag).is_none());
        self.update_buffer_config(id).expect("new buffer must always have config")
    }

    /// Updates the default language for the given buffer.
    ///
    /// # Panics:
    ///
    /// Panics if `id` does not exist.
    pub(crate) fn update_buffer_path(&mut self, id: BufferId, path: &Path) -> Option<Table> {
        assert!(self.buffer_tags.contains_key(&id));
        let lang = self.language_for_path(path).unwrap_or_default();
        let has_changed = self.buffer_tags.get_mut(&id).map(|tag| tag.set_detected(lang)).unwrap();

        if has_changed {
            self.update_buffer_config(id)
        } else {
            None
        }
    }

    /// Instructs the `ConfigManager` to stop tracking a given buffer.
    ///
    /// # Panics:
    ///
    /// Panics if `id` does not exist.
    pub(crate) fn remove_buffer(&mut self, id: BufferId) {
        self.buffer_tags.remove(&id).expect("remove key must exist");
        self.buffer_configs.remove(&id);
        // TODO: remove any overrides
    }

    /// Sets a specific language for the given buffer. This is used if the
    /// user selects a specific language in the frontend, for instance.
    pub(crate) fn override_language(
        &mut self,
        id: BufferId,
        new_lang: LanguageId,
    ) -> Option<Table> {
        let has_changed = self
            .buffer_tags
            .get_mut(&id)
            .map(|tag| tag.set_user(Some(new_lang)))
            .expect("buffer must exist");
        if has_changed {
            self.update_buffer_config(id)
        } else {
            None
        }
    }

    fn update_buffer_config(&mut self, id: BufferId) -> Option<Table> {
        let new_config = self.generate_buffer_config(id);
        let changes = new_config.changes_from(self.buffer_configs.get(&id));
        self.buffer_configs.insert(id, new_config);
        changes
    }

    fn update_all_buffer_configs(&mut self) -> Vec<(BufferId, Table)> {
        self.buffer_configs
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .flat_map(|k| self.update_buffer_config(k).map(|c| (k, c)))
            .collect::<Vec<_>>()
    }

    fn generate_buffer_config(&mut self, id: BufferId) -> BufferConfig {
        // it's possible for a buffer to be tagged with since-removed language
        let lang = self
            .buffer_tags
            .get(&id)
            .map(LanguageTag::resolve)
            .and_then(|name| self.languages.language_for_name(name))
            .map(|l| l.name.clone());
        let mut configs = Vec::new();

        configs.push(self.configs.get(&ConfigDomain::General));
        if let Some(s) = lang {
            configs.push(self.configs.get(&s.into()))
        };
        configs.push(self.configs.get(&ConfigDomain::SysOverride(id)));
        configs.push(self.configs.get(&ConfigDomain::UserOverride(id)));

        let configs = configs
            .iter()
            .flat_map(Option::iter)
            .map(|c| c.cache.clone())
            .rev()
            .collect::<Vec<_>>();

        let stack = TableStack(configs);
        stack.into_config()
    }

    /// Returns a reference to the `BufferConfig` for this buffer.
    ///
    /// # Panics:
    ///
    /// Panics if `id` does not exist. The caller is responsible for ensuring
    /// that the `ConfigManager` is kept up to date as buffers are added/removed.
    pub(crate) fn get_buffer_config(&self, id: BufferId) -> &BufferConfig {
        self.buffer_configs.get(&id).unwrap()
    }

    /// Returns the language associated with this buffer.
    ///
    /// # Panics:
    ///
    /// Panics if `id` does not exist.
    pub(crate) fn get_buffer_language(&self, id: BufferId) -> LanguageId {
        self.buffer_tags.get(&id).map(LanguageTag::resolve).unwrap()
    }

    /// Set the available `LanguageDefinition`s. Overrides any previous values.
    pub fn set_languages(&mut self, languages: Languages) {
        // remove base configs for any removed languages
        self.languages.difference(&languages).iter().for_each(|lang| {
            let domain: ConfigDomain = lang.name.clone().into();
            if let Some(pair) = self.configs.get_mut(&domain) {
                *pair = pair.new_with_base(None);
            }
        });

        for language in languages.iter() {
            let lang_id = language.name.clone();
            let domain: ConfigDomain = lang_id.into();
            let default_config = language.default_config.clone();
            self.configs
                .entry(domain.clone())
                .and_modify(|c| *c = c.new_with_base(default_config.clone()))
                .or_insert_with(|| ConfigPair::with_base(default_config));
            if let Some(table) = self.load_user_config_file(&domain) {
                // we can't report this error because we don't have a
                // handle to the peer :|
                let _ = self.set_user_config(domain, table);
            }
        }
        //FIXME these changes are happening silently, which won't work once
        //languages can by dynamically changed
        self.languages = languages;
        self.update_all_buffer_configs();
    }

    fn load_user_config_file(&self, domain: &ConfigDomain) -> Option<Table> {
        let path = self
            .config_dir
            .as_ref()
            .map(|p| p.join(domain.file_stem()).with_extension("xiconfig"))?;

        if !path.exists() {
            return None;
        }

        match try_load_from_file(&path) {
            Ok(t) => Some(t),
            Err(e) => {
                error!("Error loading config: {:?}", e);
                None
            }
        }
    }

    pub fn language_for_path(&self, path: &Path) -> Option<LanguageId> {
        self.languages.language_for_path(path).map(|lang| lang.name.clone())
    }

    /// Sets the config for the given domain, removing any existing config.
    /// Returns a `Vec` of individual buffer config changes that result from
    /// this update, or a `ConfigError` if `config` is poorly formed.
    pub fn set_user_config(
        &mut self,
        domain: ConfigDomain,
        config: Table,
    ) -> Result<Vec<(BufferId, Table)>, ConfigError> {
        self.check_table(&config)?;
        self.configs.entry(domain).or_insert_with(|| ConfigPair::with_base(None)).set_table(config);
        Ok(self.update_all_buffer_configs())
    }

    /// Returns the `Table` produced by applying `changes` to the current user
    /// config for the given `ConfigDomain`.
    ///
    /// # Note:
    ///
    /// When the user modifys a config _file_, the whole file is read,
    /// and we can just overwrite any existing user config with the newly
    /// loaded one.
    ///
    /// When the client modifies a config via the RPC mechanism, however,
    /// this isn't the case. Instead of sending all config settings with
    /// each update, the client just sends the keys/values they would like
    /// to change. When they would like to remove a previously set key,
    /// they send `Null` as the value for that key.
    ///
    /// This function creates a new table which is the product of updating
    /// any existing table by applying the client's changes. This new table can
    /// then be passed to `Self::set_user_config(..)`, as if it were loaded
    /// from disk.
    pub(crate) fn table_for_update(&mut self, domain: ConfigDomain, changes: Table) -> Table {
        self.configs
            .entry(domain)
            .or_insert_with(|| ConfigPair::with_base(None))
            .table_for_update(changes)
    }

    /// Returns the `ConfigDomain` relevant to a given file, if one exists.
    pub fn domain_for_path(&self, path: &Path) -> Option<ConfigDomain> {
        if path.extension().map(|e| e != "xiconfig").unwrap_or(true) {
            return None;
        }
        match path.file_stem().and_then(|s| s.to_str()) {
            Some("preferences") => Some(ConfigDomain::General),
            Some(name) if self.languages.language_for_name(&name).is_some() => {
                let lang =
                    self.languages.language_for_name(&name).map(|lang| lang.name.clone()).unwrap();
                Some(ConfigDomain::Language(lang))
            }
            //TODO: plugin configs
            _ => None,
        }
    }

    fn check_table(&self, table: &Table) -> Result<(), ConfigError> {
        let defaults = self
            .configs
            .get(&ConfigDomain::General)
            .and_then(|pair| pair.base.clone())
            .expect("general domain must have defaults");
        let mut defaults: Table = defaults.as_ref().clone();
        for (k, v) in table.iter() {
            // changes can include 'null', which means clear field
            if v.is_null() {
                continue;
            }
            defaults.insert(k.to_owned(), v.to_owned());
        }
        let _: BufferItems = serde_json::from_value(defaults.into())?;
        Ok(())
    }

    /// Path to themes sub directory inside config directory.
    /// Creates one if not present.
    pub(crate) fn get_themes_dir(&self) -> Option<PathBuf> {
        let themes_dir = self.config_dir.as_ref().map(|p| p.join("themes"));

        if let Some(p) = themes_dir {
            if p.exists() {
                return Some(p);
            }
            if fs::DirBuilder::new().create(&p).is_ok() {
                return Some(p);
            }
        }
        None
    }

    /// Path to plugins sub directory inside config directory.
    /// Creates one if not present.
    pub(crate) fn get_plugins_dir(&self) -> Option<PathBuf> {
        let plugins_dir = self.config_dir.as_ref().map(|p| p.join("plugins"));

        if let Some(p) = plugins_dir {
            if p.exists() {
                return Some(p);
            }
            if fs::DirBuilder::new().create(&p).is_ok() {
                return Some(p);
            }
        }
        None
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
    where
        for<'de> T: Deserialize<'de>,
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
                return Some(v);
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

impl ConfigDomain {
    fn file_stem(&self) -> &str {
        match self {
            ConfigDomain::General => "preferences",
            ConfigDomain::Language(lang) => lang.as_ref(),
            ConfigDomain::UserOverride(_) | ConfigDomain::SysOverride(_) => "we don't have files",
        }
    }
}

impl LanguageTag {
    fn new(detected: LanguageId) -> Self {
        LanguageTag { detected, user: None }
    }

    fn resolve(&self) -> LanguageId {
        self.user.as_ref().unwrap_or(&self.detected).clone()
    }

    /// Set the detected language. Returns `true` if this changes the resolved
    /// language.
    fn set_detected(&mut self, detected: LanguageId) -> bool {
        let before = self.resolve();
        self.detected = detected;
        before != self.resolve()
    }

    /// Set the user-specified language. Returns `true` if this changes
    /// the resolved language.
    #[allow(dead_code)]
    fn set_user(&mut self, new_lang: Option<LanguageId>) -> bool {
        let has_changed = self.user != new_lang;
        self.user = new_lang;
        has_changed
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
            UnknownDomain(ref s) => write!(f, "UnknownDomain: {}", s),
            Parse(ref p, ref e) => write!(f, "Parse ({:?}), {}", p, e),
            Io(ref e) => write!(f, "error loading config: {}", e),
            UnexpectedItem(ref e) => write!(f, "{}", e),
        }
    }
}

impl Error for ConfigError {}

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
pub(crate) fn init_config_dir(dir: &Path) -> io::Result<()> {
    let builder = fs::DirBuilder::new();
    builder.create(dir)?;
    builder.create(dir.join("plugins"))?;
    Ok(())
}

/// Attempts to load a config from a file. The config's domain is determined
/// by the file name.
pub(crate) fn try_load_from_file(path: &Path) -> Result<Table, ConfigError> {
    let mut file = fs::File::open(&path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    table_from_toml_str(&contents).map_err(|e| ConfigError::Parse(path.to_owned(), e))
}

pub(crate) fn table_from_toml_str(s: &str) -> Result<Table, toml::de::Error> {
    let table = toml::from_str(&s)?;
    let table = from_toml_value(table).as_object().unwrap().to_owned();
    Ok(table)
}

//adapted from https://docs.rs/crate/config/0.7.0/source/src/file/format/toml.rs
/// Converts between toml (used to write config files) and json
/// (used to store config values internally).
fn from_toml_value(value: toml::Value) -> Value {
    match value {
        toml::Value::String(value) => value.into(),
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
    use crate::syntax::LanguageDefinition;

    #[test]
    fn test_overrides() {
        let user_config = table_from_toml_str(r#"tab_size = 42"#).unwrap();
        let rust_config = table_from_toml_str(r#"tab_size = 31"#).unwrap();

        let lang_def = rust_lang_def(None);
        let rust_id: LanguageId = "Rust".into();

        let buf_id_1 = BufferId(1); // no language
        let buf_id_2 = BufferId(2); // just rust
        let buf_id_3 = BufferId(3); // rust, + system overrides

        let mut manager = ConfigManager::new(None, None);
        manager.set_languages(Languages::new(&[lang_def]));
        manager.set_user_config(rust_id.clone().into(), rust_config).unwrap();
        manager.set_user_config(ConfigDomain::General, user_config).unwrap();

        let changes = json!({"tab_size": 67}).as_object().unwrap().to_owned();
        manager.set_user_config(ConfigDomain::SysOverride(buf_id_3), changes).unwrap();

        manager.add_buffer(buf_id_1, None);
        manager.add_buffer(buf_id_2, Some(Path::new("file.rs")));
        manager.add_buffer(buf_id_3, Some(Path::new("file2.rs")));

        // system override
        let config = manager.get_buffer_config(buf_id_1).to_owned();
        assert_eq!(config.source.0.len(), 1);
        assert_eq!(config.items.tab_size, 42);
        let config = manager.get_buffer_config(buf_id_2).to_owned();
        assert_eq!(config.items.tab_size, 31);
        let config = manager.get_buffer_config(buf_id_3).to_owned();
        assert_eq!(config.items.tab_size, 67);

        // user override trumps everything
        let changes = json!({"tab_size": 85}).as_object().unwrap().to_owned();
        manager.set_user_config(ConfigDomain::UserOverride(buf_id_3), changes).unwrap();
        let config = manager.get_buffer_config(buf_id_3);
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
        let buf_id = BufferId(1);
        manager.add_buffer(buf_id, None);
        assert_eq!(manager.get_buffer_config(buf_id).items.font_size, 14.);
        let changes = json!({"font_size": 69, "font_face": "nice"}).as_object().unwrap().to_owned();
        let table = manager.table_for_update(ConfigDomain::General, changes);
        manager.set_user_config(ConfigDomain::General, table).unwrap();
        assert_eq!(manager.get_buffer_config(buf_id).items.font_size, 69.);

        // null values in updates removes keys
        let changes = json!({ "font_size": Value::Null }).as_object().unwrap().to_owned();
        let table = manager.table_for_update(ConfigDomain::General, changes);
        manager.set_user_config(ConfigDomain::General, table).unwrap();
        assert_eq!(manager.get_buffer_config(buf_id).items.font_size, 14.);
        assert_eq!(manager.get_buffer_config(buf_id).items.font_face, "nice");
    }

    #[test]
    fn lang_overrides() {
        let mut manager = ConfigManager::new(None, None);
        let lang_defaults = json!({"font_size": 69, "font_face": "nice"});
        let lang_overrides = json!({"font_size": 420, "font_face": "cool"});
        let lang_def = rust_lang_def(lang_defaults.as_object().map(Table::clone));
        let lang_id: LanguageId = "Rust".into();
        let domain: ConfigDomain = lang_id.clone().into();

        manager.set_languages(Languages::new(&[lang_def.clone()]));
        assert_eq!(manager.languages.iter().count(), 1);

        let buf_id = BufferId(1);
        manager.add_buffer(buf_id, Some(Path::new("file.rs")));

        let config = manager.get_buffer_config(buf_id).to_owned();
        assert_eq!(config.source.0.len(), 2);
        assert_eq!(config.items.font_size, 69.);

        // removing language should remove default configs
        manager.set_languages(Languages::new(&[]));
        assert_eq!(manager.languages.iter().count(), 0);

        let config = manager.get_buffer_config(buf_id).to_owned();
        assert_eq!(config.source.0.len(), 1);
        assert_eq!(config.items.font_size, 14.);

        manager
            .set_user_config(domain.clone(), lang_overrides.as_object().map(Table::clone).unwrap())
            .unwrap();

        // user config for unknown language is ignored
        let config = manager.get_buffer_config(buf_id).to_owned();
        assert_eq!(config.items.font_size, 14.);

        // user config trumps defaults when language exists
        manager.set_languages(Languages::new(&[lang_def.clone()]));
        let config = manager.get_buffer_config(buf_id).to_owned();
        assert_eq!(config.items.font_size, 420.);

        let changes = json!({ "font_size": Value::Null }).as_object().unwrap().to_owned();

        // null key should void user setting, leave language default
        let table = manager.table_for_update(domain.clone(), changes);
        manager.set_user_config(domain.clone(), table).unwrap();
        let config = manager.get_buffer_config(buf_id).to_owned();
        assert_eq!(config.items.font_size, 69.);

        manager.set_languages(Languages::new(&[]));
        let config = manager.get_buffer_config(buf_id);
        assert_eq!(config.items.font_size, 14.);
    }

    fn rust_lang_def<T: Into<Option<Table>>>(defaults: T) -> LanguageDefinition {
        LanguageDefinition::simple("Rust", &["rs"], "source.rust", defaults.into())
    }
}
