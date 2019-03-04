// Copyright 2018 The xi-editor Authors.
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

//! Management of annotations.

use serde_json::Value;
use std::collections::HashMap;

use crate::plugins::PluginId;
use crate::view::View;
use crate::xi_rope::spans::{Spans, SpansBuilder};
use crate::xi_rope::{Interval, Rope};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum AnnotationType {
    Selection,
    Find,
    Other(String),
}

impl AnnotationType {
    fn as_str(&self) -> &str {
        match self {
            AnnotationType::Find => "find",
            AnnotationType::Selection => "selection",
            AnnotationType::Other(ref s) => s,
        }
    }
}

/// Location and range of an annotation ([start_line, start_col, end_line, end_col]).
pub type AnnotationRange = Vec<[usize; 4]>;

/// A set of annotations of a given type.
#[derive(Clone, Debug)]
pub struct Annotations {
    pub items: Spans<Value>,
    pub annotation_type: AnnotationType,
}

impl Annotations {
    pub fn update(&mut self, interval: Interval, items: Spans<Value>) {
        self.items.edit(interval, items);
    }
}

/// A region of an `Annotation`.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AnnotationSlice {
    annotation_type: AnnotationType,
    /// Annotation occurrences, guaranteed non-descending start order.
    ranges: AnnotationRange,
    /// If present, one payload per range.
    payloads: Option<Vec<Value>>,
}

impl AnnotationSlice {
    pub fn new(
        annotation_type: AnnotationType,
        ranges: AnnotationRange,
        payloads: Option<Vec<Value>>,
    ) -> Self {
        AnnotationSlice { annotation_type, ranges, payloads }
    }

    /// Returns json representation.
    pub fn to_json(&self) -> Value {
        json!({
            "type": self.annotation_type.as_str(),
            "ranges": self.ranges,
            "payloads": self.payloads,
            "n": self.ranges.len()
        })
    }

    pub fn to_annotations(&self, view: &View, text: &Rope) -> Annotations {
        let last_entry = self.ranges.last().unwrap_or(&[0, 0, 0, 0]);
        let mut sb = SpansBuilder::new(last_entry[2] + last_entry[3]);

        for (i, &range) in self.ranges.iter().enumerate() {
            let payload = match &self.payloads {
                Some(p) => p[i].clone(),
                None => json!(null),
            };

            let start = view.offset_of_line(text, range[0]) + range[1];
            let end = view.offset_of_line(text, range[2]) + range[3];

            sb.add_span(Interval::new(start, end), payload);
        }

        Annotations { items: sb.build(), annotation_type: self.annotation_type.clone() }
    }
}

/// A trait for types (like `Selection`) that have a distinct representation
/// in core but are presented to the frontend as annotations.
pub trait ToAnnotation {
    /// Returns annotations that overlap the provided interval.
    fn get_annotations(&self, interval: Interval, view: &View, text: &Rope) -> AnnotationSlice;
}

/// All the annotations for a given view
pub struct AnnotationStore {
    store: HashMap<PluginId, Vec<Annotations>>,
}

impl AnnotationStore {
    pub fn new() -> Self {
        AnnotationStore { store: HashMap::new() }
    }

    /// Invalidates and removes all annotations in the range of the interval.
    pub fn invalidate(&mut self, _interval: Interval) {
        // todo
    }

    /// Applies an update from a plugin to a set of annotations
    pub fn update(&mut self, source: PluginId, interval: Interval, item: Annotations) {
        let updated_items = item.clone();
        self.store
            .entry(source)
            .and_modify(|e| {
                let mut annotations = e
                    .iter()
                    .filter(|a| a.annotation_type != updated_items.annotation_type)
                    .cloned()
                    .collect::<Vec<Annotations>>();

                match e.into_iter().find(|a| a.annotation_type == updated_items.annotation_type) {
                    Some(outdated_annotations) => {
                        let mut updated_annotations = outdated_annotations.clone();
                        updated_annotations.update(interval, updated_items.items);
                        annotations.push(updated_annotations);
                    }
                    None => {
                        annotations.push(updated_items);
                    }
                }

                *e = annotations;
            })
            .or_insert(vec![item]);
    }

    /// Returns an iterator which produces, for each type of annotation,
    /// those annotations which intersect the given interval.
    pub fn iter_range<'c>(
        &'c self,
        view: &'c View,
        text: &'c Rope,
        interval: Interval,
    ) -> impl Iterator<Item = AnnotationSlice> + 'c {

        self.store.iter().flat_map(move |(_plugin, value)| {
            value.iter().map(move |annotation| {

                let payloads = annotation
                    .items
                    .subseq(interval)
                    .iter()
                    .map(|(_i, p)| p.clone())
                    .collect::<Vec<Value>>();

                let ranges = annotation
                    .items
                    .subseq(interval)
                    .iter()
                    .map(|(i, _p)| {
                        let (start_line, start_col) = view.offset_to_line_col(text, i.start());
                        let (end_line, end_col) = view.offset_to_line_col(text, i.end());
                        [start_line, start_col, end_line, end_col]
                    })
                    .collect::<AnnotationRange>();

                AnnotationSlice {
                    annotation_type: annotation.annotation_type.clone(),
                    ranges,
                    payloads: Some(payloads),
                }
            })
        })
    }

    /// Removes any annotations provided by this plugin
    pub fn clear(&mut self, plugin: PluginId) {
        self.store.remove(&plugin);
    }
}
