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

//! Management of styles.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{self, Value};
use syntect::dumps::{dump_to_file, from_dump_file};
use syntect::highlighting::StyleModifier as SynStyleModifier;
use syntect::highlighting::{Color, Highlighter, Theme, ThemeSet};
use syntect::LoadingError;

pub use syntect::highlighting::ThemeSettings;

pub const N_RESERVED_STYLES: usize = 8;
const SYNTAX_PRIORITY_DEFAULT: u16 = 200;
const SYNTAX_PRIORITY_LOWEST: u16 = 0;
pub const DEFAULT_THEME: &str = "InspiredGitHub";

#[derive(Clone, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
/// A mergeable style. All values except priority are optional.
///
/// Note: A `None` value represents the absense of preference; in the case of
/// boolean options, `Some(false)` means that this style will override a lower
/// priority value in the same field.
pub struct Style {
    /// The priority of this style, in the range (0, 1000). Used to resolve
    /// conflicting fields when merging styles. The higher priority wins.
    #[serde(skip_serializing)]
    pub priority: u16,
    /// The foreground text color, in ARGB.
    pub fg_color: Option<u32>,
    /// The background text color, in ARGB.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bg_color: Option<u32>,
    /// The font-weight, in the range 100-900, interpreted like the CSS
    /// font-weight property.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
}

impl Style {
    /// Creates a new `Style` by converting from a `Syntect::StyleModifier`.
    pub fn from_syntect_style_mod(style: &SynStyleModifier) -> Self {
        let font_style = style.font_style.map(|s| s.bits()).unwrap_or_default();
        let weight = if (font_style & 1) != 0 { Some(700) } else { None };
        let underline = if (font_style & 2) != 0 { Some(true) } else { None };
        let italic = if (font_style & 4) != 0 { Some(true) } else { None };

        Self::new(
            SYNTAX_PRIORITY_DEFAULT,
            style.foreground.map(Self::rgba_from_syntect_color),
            style.background.map(Self::rgba_from_syntect_color),
            weight,
            underline,
            italic,
        )
    }

    pub fn new<O32, O16, OB>(
        priority: u16,
        fg_color: O32,
        bg_color: O32,
        weight: O16,
        underline: OB,
        italic: OB,
    ) -> Self
    where
        O32: Into<Option<u32>>,
        O16: Into<Option<u16>>,
        OB: Into<Option<bool>>,
    {
        assert!(priority <= 1000);
        Style {
            priority,
            fg_color: fg_color.into(),
            bg_color: bg_color.into(),
            weight: weight.into(),
            underline: underline.into(),
            italic: italic.into(),
        }
    }

    /// Returns the default style for the given `Theme`.
    pub fn default_for_theme(theme: &Theme) -> Self {
        let fg = theme.settings.foreground.unwrap_or(Color::BLACK);
        Style::new(
            SYNTAX_PRIORITY_LOWEST,
            Some(Self::rgba_from_syntect_color(fg)),
            None,
            None,
            None,
            None,
        )
    }

    /// Creates a new style by combining attributes of `self` and `other`.
    /// If both styles define an attribute, the highest priority wins; `other`
    /// wins in the case of a tie.
    ///
    /// Note: when merging multiple styles, apply them in increasing priority.
    pub fn merge(&self, other: &Style) -> Style {
        let (p1, p2) = if self.priority > other.priority { (self, other) } else { (other, self) };

        Style::new(
            p1.priority,
            p1.fg_color.or(p2.fg_color),
            p1.bg_color.or(p2.bg_color),
            p1.weight.or(p2.weight),
            p1.underline.or(p2.underline),
            p1.italic.or(p2.italic),
        )
    }

    /// Encode this `Style`, setting the `id` property.
    ///
    /// Note: this should only be used when sending the `def_style` RPC.
    pub fn to_json(&self, id: usize) -> Value {
        let mut as_val = serde_json::to_value(self).expect("failed to encode style");
        as_val["id"] = id.into();
        as_val
    }

    fn rgba_from_syntect_color(color: Color) -> u32 {
        let Color { r, g, b, a } = color;
        ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
    }
}

/// A map from styles to client identifiers for a given `Theme`.
pub struct ThemeStyleMap {
    themes: ThemeSet,
    theme_name: String,
    theme: Theme,

    // Keeps a list of default themes.
    default_themes: Vec<String>,
    default_style: Style,
    map: HashMap<Style, usize>,

    // Maintains the map of found themes and their paths.
    path_map: BTreeMap<String, PathBuf>,

    // It's not obvious we actually have to store the style, we seem to only need it
    // as the key in the map.
    styles: Vec<Style>,
    themes_dir: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
    caching_enabled: bool,
}

impl ThemeStyleMap {
    pub fn new(themes_dir: Option<PathBuf>) -> ThemeStyleMap {
        let themes = ThemeSet::load_defaults();
        let theme_name = DEFAULT_THEME.to_owned();
        let theme = themes.themes.get(&theme_name).expect("missing theme").to_owned();
        let default_themes = themes.themes.keys().cloned().collect();
        let default_style = Style::default_for_theme(&theme);
        let cache_dir = None;
        let caching_enabled = true;

        ThemeStyleMap {
            themes,
            theme_name,
            theme,
            default_themes,
            default_style,
            map: HashMap::new(),
            path_map: BTreeMap::new(),
            styles: Vec::new(),
            themes_dir,
            cache_dir,
            caching_enabled,
        }
    }

    pub fn get_default_style(&self) -> &Style {
        &self.default_style
    }

    pub fn get_highlighter(&self) -> Highlighter {
        Highlighter::new(&self.theme)
    }

    pub fn get_theme_name(&self) -> &str {
        &self.theme_name
    }

    pub fn get_theme_settings(&self) -> &ThemeSettings {
        &self.theme.settings
    }

    pub fn get_theme_names(&self) -> Vec<String> {
        self.path_map.keys().chain(self.default_themes.iter()).cloned().collect()
    }

    pub fn contains_theme(&self, k: &str) -> bool {
        self.themes.themes.contains_key(k)
    }

    pub fn set_theme(&mut self, theme_name: &str) -> Result<(), &'static str> {
        match self.load_theme(theme_name) {
            Ok(()) => {
                if let Some(new_theme) = self.themes.themes.get(theme_name) {
                    self.theme = new_theme.to_owned();
                    self.theme_name = theme_name.to_owned();
                    self.default_style = Style::default_for_theme(&self.theme);
                    self.map = HashMap::new();
                    self.styles = Vec::new();
                    Ok(())
                } else {
                    Err("unknown theme")
                }
            }
            Err(e) => {
                error!("Encountered error {:?} while trying to load {:?}", e, theme_name);
                Err("could not load theme")
            }
        }
    }

    pub fn merge_with_default(&self, style: &Style) -> Style {
        self.default_style.merge(style)
    }

    pub fn lookup(&self, style: &Style) -> Option<usize> {
        self.map.get(style).cloned()
    }

    pub fn add(&mut self, style: &Style) -> usize {
        let result = self.styles.len() + N_RESERVED_STYLES;
        self.map.insert(style.clone(), result);
        self.styles.push(style.clone());
        result
    }

    /// Delete key and the corresponding dump file from the themes map.
    pub(crate) fn remove_theme(&mut self, path: &Path) -> Option<String> {
        validate_theme_file(path).ok()?;

        let theme_name = path.file_stem().and_then(OsStr::to_str)?;
        self.themes.themes.remove(theme_name);
        self.path_map.remove(theme_name);

        let dump_p = self.get_dump_path(theme_name)?;
        if dump_p.exists() {
            let _ = fs::remove_file(dump_p);
        }

        Some(theme_name.to_string())
    }

    /// Cache all themes names and their paths inside the given directory.
    pub(crate) fn load_theme_dir(&mut self) {
        if let Some(themes_dir) = self.themes_dir.clone() {
            match ThemeSet::discover_theme_paths(themes_dir) {
                Ok(themes) => {
                    self.caching_enabled = self.caching_enabled && self.init_cache_dir();
                    // We look through the theme folder here and cache their names/paths to a
                    // path hashmap.
                    for theme_p in &themes {
                        match self.load_theme_info_from_path(theme_p) {
                            Ok(_) => (),
                            Err(e) => {
                                error!("Encountered error {:?} loading theme at {:?}", e, theme_p)
                            }
                        }
                    }
                }
                Err(e) => error!("Error loading themes dir: {:?}", e),
            }
        }
    }

    /// A wrapper around `from_dump_file`
    /// to validate the state of dump file.
    /// Invalidates if mod time of dump is less
    /// than the original one.
    fn try_load_from_dump(&self, theme_p: &Path) -> Option<(String, Theme)> {
        if !self.caching_enabled {
            return None;
        }

        let theme_name = theme_p.file_stem().and_then(OsStr::to_str)?;

        let dump_p = self.get_dump_path(theme_name)?;

        if !&dump_p.exists() {
            return None;
        }

        //NOTE: `try_load_from_dump` will return `None` if the file at
        //`dump_p` or `theme_p` is deleted before the execution of this fn.
        let mod_t = fs::metadata(&dump_p).and_then(|md| md.modified()).ok()?;
        let mod_t_orig = fs::metadata(theme_p).and_then(|md| md.modified()).ok()?;

        if mod_t >= mod_t_orig {
            from_dump_file(&dump_p).ok().map(|t| (theme_name.to_owned(), t))
        } else {
            // Delete dump file
            let _ = fs::remove_file(&dump_p);
            None
        }
    }

    /// Loads a theme's name and its respective path into the theme path map.
    pub(crate) fn load_theme_info_from_path(
        &mut self,
        theme_p: &Path,
    ) -> Result<String, LoadingError> {
        validate_theme_file(theme_p)?;
        let theme_name =
            theme_p.file_stem().and_then(OsStr::to_str).ok_or(LoadingError::BadPath)?;

        self.path_map.insert(theme_name.to_string(), theme_p.to_path_buf());

        Ok(theme_name.to_owned())
    }

    /// Loads theme using syntect's `get_theme` fn to our `theme` path map.
    /// Stores binary dump in a file with `tmdump` extension, only if
    /// caching is enabled.
    fn load_theme(&mut self, theme_name: &str) -> Result<(), LoadingError> {
        // If it is the current theme (the user has edited it), we load it again.
        // Otherwise, if it's a default theme or the theme has already been loaded, we can move on.
        if self.contains_theme(theme_name) && self.get_theme_name() != theme_name {
            return Ok(());
        }
        // If we haven't loaded the theme before, we try to load it from the dump if a dump
        // exists or load it from the theme file itself.
        let theme_p = &self.path_map.get(theme_name).cloned();
        if let Some(theme_p) = theme_p {
            match self.try_load_from_dump(theme_p) {
                Some((dump_theme_name, dump_theme_data)) => {
                    self.insert_to_map(dump_theme_name, dump_theme_data);
                }
                None => {
                    let theme = ThemeSet::get_theme(theme_p)?;
                    if self.caching_enabled {
                        if let Some(dump_p) = self.get_dump_path(theme_name) {
                            let _ = dump_to_file(&theme, dump_p);
                        }
                    }
                    self.insert_to_map(theme_name.to_owned(), theme);
                }
            }
            Ok(())
        } else {
            Err(LoadingError::BadPath)
        }
    }

    fn insert_to_map(&mut self, k: String, v: Theme) {
        self.themes.themes.insert(k, v);
    }

    /// Returns dump's path corresponding to the given theme name.
    fn get_dump_path(&self, theme_name: &str) -> Option<PathBuf> {
        self.cache_dir.as_ref().map(|p| p.join(theme_name).with_extension("tmdump"))
    }

    /// Compare the stored file paths in `self.state`
    /// to the present ones.
    pub(crate) fn sync_dir(&mut self, dir: Option<&Path>) {
        if let Some(themes_dir) = dir {
            if let Ok(paths) = ThemeSet::discover_theme_paths(themes_dir) {
                let current_state: HashSet<PathBuf> = paths.into_iter().collect();
                let maintained_state: HashSet<PathBuf> = self.path_map.values().cloned().collect();

                let to_insert = current_state.difference(&maintained_state);
                for path in to_insert {
                    let _ = self.load_theme_info_from_path(path);
                }
                let to_remove = maintained_state.difference(&current_state);
                for path in to_remove {
                    self.remove_theme(path);
                }
            }
        }
    }
    /// Creates the cache dir returns true
    /// if it is successfully initialized or
    /// already exists.
    fn init_cache_dir(&mut self) -> bool {
        self.cache_dir = self.themes_dir.clone().map(|p| p.join("cache"));

        if let Some(ref p) = self.cache_dir {
            if p.exists() {
                return true;
            }
            fs::DirBuilder::new().create(&p).is_ok()
        } else {
            false
        }
    }
}

/// Used to remove files with extension other than `tmTheme`.
fn validate_theme_file(path: &Path) -> Result<bool, LoadingError> {
    path.extension().map(|e| e != "tmTheme").ok_or(LoadingError::BadPath)
}

impl fmt::Debug for Style {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fn fmt_color(f: &mut fmt::Formatter, c: Option<u32>) -> fmt::Result {
            if let Some(c) = c {
                write!(f, "#{:X}", c)
            } else {
                write!(f, "None")
            }
        }

        write!(f, "Style( P{}, fg: ", self.priority)?;
        fmt_color(f, self.fg_color)?;
        write!(f, " bg: ")?;
        fmt_color(f, self.bg_color)?;

        if let Some(w) = self.weight {
            write!(f, " weight {}", w)?;
        }
        if let Some(i) = self.italic {
            write!(f, " ital: {}", i)?;
        }
        if let Some(u) = self.underline {
            write!(f, " uline: {}", u)?;
        }
        write!(f, " )")
    }
}
