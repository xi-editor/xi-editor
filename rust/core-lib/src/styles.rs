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

use serde_json::value::Value;

const N_RESERVED_STYLES: usize = 1;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Style {
    pub fg: u32,  // ARGB format
    pub bg: u32,  // ARGB format
    pub weight: u16,  // 100-900, same interpretation as CSS
    pub underline: bool,
    pub italic: bool,
}

impl Style {
    // construct the params for a def_style request
    pub fn to_json(&self, id: usize) -> Value {
        let mut json = json!({
            "id": id,
            "fg_color": self.fg,
        });

        if (self.bg >> 24) > 0 {
            json["bg_color"] = json!(self.bg);
        }
        if self.weight != 400 {
            json["weight"] = json!(self.weight);
        }
        if self.underline {
            json["underline"] = json!(self.underline);
        }
        if self.italic {
            json["italic"] = json!(self.italic);
        }
        json 
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
