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

//! Management of styles.

use std::collections::HashMap;

use serde_json::{self, Value};
use syntect::highlighting::StyleModifier as SynStyleModifier;
use syntect::highlighting::{Color, Theme, BLACK};

const N_RESERVED_STYLES: usize = 2;
const SYNTAX_PRIORITY_DEFAULT: u16 = 200;
const SYNTAX_PRIORITY_LOWEST: u16 = 0;

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
            style.background.map(|c| Self::rgba_from_syntect_color(&c)),
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
            priority: priority,
            fg_color: fg_color.into(),
            bg_color: bg_color.into(),
            weight: weight.into(),
            underline: underline.into(),
            italic: italic.into(),
        }
    }

    /// Returns the default style for the given `Theme`.
    pub fn default_for_theme(theme: &Theme) -> Self {
        let fg = theme.settings.foreground.unwrap_or(BLACK);
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
        as_val["id"] = serde_json::to_value(id).unwrap();
        as_val
    }

    fn rgba_from_syntect_color(color: &Color) -> u32 {
        let &Color { r, g, b, a } = color;
        ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
    }
}

pub struct StyleMap {
    map: HashMap<Style, usize>,

    // It's not obvious we actually have to store the style, we seem to only need it
    // as the key in the map.
    styles: Vec<Style>,
}

impl StyleMap {
    pub fn new() -> StyleMap {
        StyleMap {
            map: HashMap::new(),
            styles: Vec::new(),
        }
    }

    pub fn lookup(&self, style: &Style) -> Option<usize> {
        self.map.get(style).map(|&ix| ix)
    }

    pub fn add(&mut self, style: &Style) -> usize {
        let result = self.styles.len() + N_RESERVED_STYLES;
        self.map.insert(style.clone(), result);
        self.styles.push(style.clone());
        result
    }
}
