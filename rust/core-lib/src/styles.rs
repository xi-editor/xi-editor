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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::ffi::OsStr;
use std::fs;

use serde_json::{self, Value};
use syntect::highlighting::StyleModifier as SynStyleModifier;
use syntect::highlighting::{Color, Theme, ThemeSet, Highlighter};
use syntect::dumps::{from_dump_file, dump_to_file};

pub use syntect::highlighting::ThemeSettings;

const N_RESERVED_STYLES: usize = 2;
const SYNTAX_PRIORITY_DEFAULT: u16 = 200;
const SYNTAX_PRIORITY_LOWEST: u16 = 0;
pub const DEFAULT_THEME: &str = "InspiredGitHub";

#[derive(Clone, PartialEq, Eq, Default, Hash, Debug, Serialize, Deserialize)]
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
        let font_style = style.font_style.map(|s|s.bits()).unwrap_or_default();
        let weight = if (font_style & 1) != 0 { Some(700) } else { None };
        let underline = if (font_style & 2) != 0 { Some(true) } else { None };
        let italic = if (font_style & 4) != 0 { Some(true) } else { None };

        Self::new(
            SYNTAX_PRIORITY_DEFAULT,
            style.foreground.map(|c| Self::rgba_from_syntect_color(&c)),
            None,
            //TODO: stop ignoring background color
            //style.background.map(|c| Self::rgba_from_syntect_color(&c)),
            weight,
            underline,
            italic,
            )
    }

    pub fn new<O32, O16, OB>(priority: u16, fg_color: O32, bg_color: O32,
                             weight: O16, underline: OB, italic: OB) -> Self
        where O32: Into<Option<u32>>,
              O16: Into<Option<u16>>,
              OB: Into<Option<bool>>
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
            Some(Self::rgba_from_syntect_color(&fg)),
            None,
            None,
            None,
            None)
    }

    /// Creates a new style by combining attributes of `self` and `other`.
    /// If both styles define an attribute, the highest priority wins; `other`
    /// wins in the case of a tie.
    ///
    /// Note: when merging multiple styles, apply them in increasing priority.
    pub fn merge(&self, other: &Style) -> Style {
        let (p1, p2) = if self.priority > other.priority {
            (self, other)
        } else {
            (other, self)
        };

        Style::new(
            p1.priority,
            p1.fg_color.or(p2.fg_color),
            //TODO: stop ignoring background color
            None,
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

    fn rgba_from_syntect_color(color: &Color) -> u32 {
        let &Color { r, g, b, a } = color;
        ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
    }
}

/// A map from styles to client identifiers for a given `Theme`.
pub struct ThemeStyleMap {
    themes: ThemeSet,
    theme_name: String,
    theme: Theme,
    default_style: Style,
    map: HashMap<Style, usize>,

    // It's not obvious we actually have to store the style, we seem to only need it
    // as the key in the map.
    styles: Vec<Style>,
}

impl ThemeStyleMap {
    pub fn new() -> ThemeStyleMap {
        let themes = ThemeSet::load_defaults();
        let theme_name = DEFAULT_THEME.to_owned();
        let theme = themes.themes.get(&theme_name).expect("missing theme").to_owned();
        let default_style = Style::default_for_theme(&theme);

        ThemeStyleMap {
            themes,
            theme_name,
            theme,
            default_style,
            map: HashMap::new(),
            styles: Vec::new(),
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

    pub fn get_theme_names(&self) -> Vec<String>  {
        self.themes.themes.keys().cloned().collect()
    }

    pub fn set_theme(&mut self, theme_name: &str) -> Result<(), &'static str> {
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

    /// Delete key from the themes map.
    pub(crate) fn remove_theme(&mut self, path: &Path) -> Option<String> {
        if let Some(theme_name) = path.file_stem().and_then(OsStr::to_str) {
            self.themes.themes.remove(theme_name);
            //Delete dump file
            let _ = fs::remove_file(path.with_extension("tmdump"));
            Some(theme_name.to_string())
        } else { None }
    }

    /// Load all themes inside the given directory.
    pub(crate) fn load_theme_dir(&mut self, themes_dir: PathBuf) {
        match ThemeSet::discover_theme_paths(themes_dir) {
            Ok(themes) => {
                for theme in themes.iter() {
                    self.load_theme(theme);
                }
            },
            Err(e) => eprintln!("Error while loading themes dir: {:?}",e),
        }
    }

    fn load_theme(&mut self, path: &Path) {
        //Check for the dump file first
        let theme_cache = path.with_extension("tmdump");
        if theme_cache.exists() {
            match from_dump_file(theme_cache) {
                Ok(theme) => self.update_theme_map(path, theme),
                Err(_) => self.insert_theme(path),
            };
        } else {
            self.insert_theme(path);
        }
    }

    /// Insert theme into the map.
    /// Store binary dump in a file with `tmdump` extension.
    pub(crate) fn insert_theme(&mut self, path: &Path) 
        -> Option<String>
    {
        match ThemeSet::get_theme(path) {
            Ok(theme) => {
                dump_to_file(&theme, path.with_extension("tmdump")).unwrap();
                self.update_theme_map(path, theme)
            },
            Err(e) => {
                eprintln!("Error while loading theme at: {:?} Err: {:?}", path, e);
                None
            }
        }
    }

    fn update_theme_map(&mut self, path: &Path, theme: Theme) 
        -> Option<String>
    {
        if let Some(theme_name) = path.file_stem().and_then(OsStr::to_str) {
            self.themes.themes.insert(theme_name.to_owned(), theme);
            Some(theme_name.to_string())
        } else { None }
    }
}