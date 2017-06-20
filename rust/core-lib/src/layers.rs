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
use syntect::parsing::Scope;

use xi_rope::interval::Interval;
use xi_rope::spans::{Spans, SpansBuilder};

use tabs::DocumentCtx;
use styles::Style;
use plugins::PluginPid;

/// A collection of layers containing scope information.
#[derive(Default)]
//TODO: rename. Probably to `Layers`
pub struct Scopes {
    layers: BTreeMap<PluginPid, ScopeLayer>,
    merged: Spans<Style>,
}

/// A collection of scope spans from a single source.
pub struct ScopeLayer {
    stack_lookup: Vec<Vec<Scope>>,
    style_lookup: Vec<Style>,
    /// Human readable scope names, for debugging
    name_lookup: Vec<Vec<String>>,
    pub spans: Spans<u32>,
}

impl Scopes {

    pub fn get_merged(&self) -> &Spans<Style> {
        &self.merged
    }

    /// Adds the provided scopes to the layer's lookup table.
    pub fn add_scopes<W: Write>(&mut self, layer: PluginPid, scopes: Vec<Vec<String>>,
                                doc_ctx: &DocumentCtx<W>) {
        self.create_if_missing(layer);
        self.layers.get_mut(&layer).unwrap().add_scopes(scopes, doc_ctx);
    }

    /// Inserts empty spans at the given interval for all layers.
    ///
    /// This is useful for clearing spans, and for updating spans
    /// as edits occur.
    pub fn update_all(&mut self, iv: Interval, len: usize) {
        self.merged.edit(iv, SpansBuilder::new(len).build());
        let empty_spans = SpansBuilder::new(len).build();
        for layer in self.layers.values_mut() {
            layer.update_spans(iv, &empty_spans);
        }
        self.resolve_styles(iv);
    }

    /// Updates the scope spans for a given layer.
    pub fn update_layer(&mut self, layer: PluginPid, iv: Interval, spans: Spans<u32>) {
        self.create_if_missing(layer);
        self.layers.get_mut(&layer).unwrap().update_spans(iv, &spans);
        self.resolve_styles(iv);
    }

    /// Removes a given layer. This will remove all styles derived from
    /// that layer's scopes.
    pub fn remove_layer(&mut self, layer: PluginPid) -> Option<ScopeLayer> {
        let layer = self.layers.remove(&layer);
        if layer.is_some() {
            let iv_all = Interval::new_closed_closed(0, self.merged.len());
            //TODO: should Spans<T> have a clear() method?
            self.merged = SpansBuilder::new(self.merged.len()).build();
            self.resolve_styles(iv_all);
        }
        layer
    }

    pub fn theme_changed<W: Write>(&mut self, doc_ctx: &DocumentCtx<W>) {
        for layer in self.layers.values_mut() {
            layer.theme_changed(doc_ctx);
        }
        self.merged = SpansBuilder::new(self.merged.len()).build();
        let iv_all = Interval::new_closed_closed(0, self.merged.len());
        self.resolve_styles(iv_all);
    }

    /// Resolves styles from all layers for the given interval, updating
    /// the master style spans.
    fn resolve_styles(&mut self, iv: Interval) {
        if self.layers.is_empty() {
            return
        }
        let mut layer_iter = self.layers.values();
        let mut resolved = layer_iter.next().unwrap().subseq_styles(iv);

        for other in layer_iter {
            //FIXME: this creates a whole lot of unnecessary styles, because
            // subseq_styles creates a new spans object and a new Style for each
            // id. Better would be to rewrite Spans::merge to be a bit more like
            // a reduce/fold operation, and then we could just use &Styles.
            let spans = other.subseq_styles(iv);
            assert_eq!(resolved.len(), spans.len());
            resolved = resolved.merge(&spans, |a, b| {
                match b {
                    Some(b) => a.merge(b),
                    None => a.to_owned(),
                }
            });
        }
        self.merged.edit(iv, resolved);
    }

    pub fn debug_print_spans(&self, iv: Interval) {
        for (id, layer) in self.layers.iter() {
            let spans = layer.spans.subseq(iv);
            print_err!("Spans for layer {:?}:", id);
            for (iv, val) in spans.iter() {
                print_err!("{}: {:?}", iv, layer.name_lookup[*val as usize])
            }
        }
    }


    fn create_if_missing(&mut self, layer_id: PluginPid) {
        if !self.layers.contains_key(&layer_id) {
            self.layers.insert(layer_id, ScopeLayer::new(self.merged.len()));
        }
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

    pub fn new(len: usize) -> Self {
        ScopeLayer {
            stack_lookup: Vec::new(),
            style_lookup: Vec::new(),
            name_lookup: Vec::new(),
            spans: SpansBuilder::new(len).build(),
        }
    }

    fn theme_changed<W: Write>(&mut self, doc_ctx: &DocumentCtx<W>) {
        // recompute styles with the new theme
        self.style_lookup = self.styles_for_stacks(self.stack_lookup.as_slice(), doc_ctx);
    }

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
            stacks.push(scopes);
            self.name_lookup.push(stack);
        }

        let mut new_styles = self.styles_for_stacks(stacks.as_slice(), doc_ctx);
        self.stack_lookup.append(&mut stacks);
        self.style_lookup.append(&mut new_styles);
    }

    fn styles_for_stacks<W: Write>(&self, stacks: &[Vec<Scope>],
                         doc_ctx: &DocumentCtx<W>) -> Vec<Style> {
        let style_map = doc_ctx.get_style_map().lock().unwrap();
        let highlighter = style_map.get_highlighter();

        let mut new_styles = Vec::new();
        for stack in stacks {
            let style = highlighter.style_mod_for_stack(stack);
            let style = Style::from_syntect_style_mod(&style);
            new_styles.push(style);
        }
        new_styles
    }

    fn update_spans(&mut self, iv: Interval, spans: &Spans<u32>) {
        self.spans.edit(iv, spans.to_owned());
    }

    /// Creates a Spans<Style> from the given interval of self.spans.
    fn subseq_styles(&self, iv: Interval) -> Spans<Style> {
        let spans = self.spans.subseq(iv);
        let mut sb = SpansBuilder::new(spans.len());
        for (iv, val) in spans.iter() {
            sb.add_span(iv, self.style_lookup[*val as usize].to_owned());
        }
        sb.build()
    }
}
