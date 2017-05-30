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

//! Handles syntax highlighting and other styling.
//!
//! NOTE: this documentation is aspirational, e.g. not all functionality is
//! currently implemented.
//!
//! Plugins provide syntax highlighting information in the form of 'scopes'.
//! Scope information originating from any number of plugins can be resolved
//! into styles using a theme, augmented with additional style definitions.

use std::collections::BTreeMap;
use std::io::Write;
use syntect::parsing::{ScopeStack, Scope};
use syntect::highlighting::Highlighter;

use xi_rope::interval::Interval;
use xi_rope::spans::{Spans, SpansBuilder};
use tabs::DocumentCtx;
use view::Style;

/// A collection of layers containing scope information.
pub struct Scopes {
    layers: BTreeMap<String, ScopeLayer>,
}

/// A collection of scope spans from a single source.
pub struct ScopeLayer {
    stack_lookup: Vec<ScopeStack>,
    style_lookup: Vec<Style>,
    /// Human readable scope names, for debugging
    name_lookup: Vec<Vec<String>>,
    pub spans: Spans<u32>,
}

impl Scopes {

    pub fn define_styles() {
        unimplemented!();
    }

    /// Adds the provided scopes to the layer's lookup table.
    pub fn add_scopes<W: Write>(&mut self, layer: &str, scopes: Vec<Vec<String>>,
                                doc_ctx: &DocumentCtx<W>) {
        self.create_if_missing(layer);
        self.layers.get_mut(layer).unwrap().add_scopes(scopes, doc_ctx);
    }

    /// Updates spans on all layers. Useful for clearing all, or OT.
    pub fn update_all(&mut self, iv: Interval, spans: Spans<u32>) {
        for layer in self.layers.values_mut() {
            layer.update_spans(iv, spans.clone());
        }
    }

    /// Updates the scope spans for a given layer.
    pub fn update_layer(&mut self, layer: &str, iv: Interval, spans: Spans<u32>) {
        self.create_if_missing(layer);
        self.layers.get_mut(layer).unwrap().update_spans(iv, spans);
    }

    /// Removes a given layer. This will remove all styles derived from
    /// that layer's scopes.
    pub fn remove_layer(&mut self, layer: &str) -> Option<ScopeLayer> {
        self.layers.remove(layer)
    }

    /// For a given Interval, generates styles from scopes, resolving conflicts.
    pub fn resolve_styles(&self, iv: Interval) -> Spans<Style> {
        if self.layers.is_empty() {
            return SpansBuilder::new(iv.size()).build()
        }
        //TODO: implement layer merging.
        let mut sb = SpansBuilder::new(iv.size());
        let layer = self.layers.values().next().unwrap();
        for (iv, val) in layer.spans.subseq(iv).iter() {
            let style = layer.style_lookup[*val as usize];
            sb.add_span(iv, style);
        }
        sb.build()
    }

    pub fn debug_print_spans(&self, iv: Interval) {
        for (id, layer) in self.layers.iter() {
            let spans = layer.spans.subseq(iv);
            print_err!("Spans for layer {}:", id);
            for (iv, val) in spans.iter() {
                print_err!("{}: {:?}", iv, layer.name_lookup[*val as usize])
            }
        }
    }

    fn create_if_missing(&mut self, layer_id: &str) {
        if !self.layers.contains_key(layer_id) {
            self.layers.insert(layer_id.to_owned(), ScopeLayer::default());
        }
    }
}

impl Default for Scopes {
    fn default() -> Self {
        Scopes { layers: BTreeMap::new() }
    }
}

impl Default for ScopeLayer {
    fn default() -> Self {
        ScopeLayer {
            stack_lookup: Vec::new(),
            style_lookup: Vec::new(),
            name_lookup: Vec::new(),
            spans: Spans::default(),
        }
    }
}

impl ScopeLayer {
    fn add_scopes<W: Write>(&mut self, scopes: Vec<Vec<String>>,
                                doc_ctx: &DocumentCtx<W>) {
        let mut stacks = Vec::with_capacity(scopes.len());
        for stack in scopes {
            let scopes = stack.iter().map(|s| Scope::new(&s))
                .filter(|result| match *result {
                   Err(ref err) => {
                       print_err!("failed to resolve scope {}\nErr: {:?}",
                                  &stack.join(" "),
                                  err);
                       false
                       }
                   _ => true
                })
            .map(|s| s.unwrap())
            .collect::<Vec<_>>();
            stacks.push(ScopeStack::from_vec(scopes));
            self.name_lookup.push(stack);
        }

        // compute styles for each new stack
        let themes = doc_ctx.theme_set.lock().unwrap();
        let theme = themes.themes.get("InspiredGitHub").expect("missing theme");
        //let theme = themes.themes.get("base16-ocean.dark").expect("missing theme");
        let highlighter = Highlighter::new(theme);

        let mut new_styles = Vec::new();
        for stack in &stacks {
            let style = highlighter.style_for_stack(&stack);
            //print_err!("new style: {:?}", style);
            let style = Style::from_syntect_style(&style);
            new_styles.push(style);
        }
        //print_err!("added styles: {:?}", &new_styles);
        self.stack_lookup.append(&mut stacks);
        self.style_lookup.append(&mut new_styles);
    }

    fn update_spans(&mut self, iv: Interval, spans: Spans<u32>) {
        self.spans.edit(iv, spans);
    }
}
