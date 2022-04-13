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

use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, SerializeSeq, Serializer};
use serde_json::{self, Value};

use std::collections::HashMap;

use crate::line_offset::LineOffset;
use crate::plugins::PluginId;
use crate::view::View;
use crate::xi_rope::spans::Spans;
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
/// Location and range of an annotation
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct AnnotationRange {
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

impl Serialize for AnnotationRange {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(4))?;
        seq.serialize_element(&self.start_line)?;
        seq.serialize_element(&self.start_col)?;
        seq.serialize_element(&self.end_line)?;
        seq.serialize_element(&self.end_col)?;
        seq.end()
    }
}

impl<'de> Deserialize<'de> for AnnotationRange {
    fn deserialize<D>(deserializer: D) -> Result<AnnotationRange, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut range = AnnotationRange { ..Default::default() };
        let seq = <[usize; 4]>::deserialize(deserializer)?;

        range.start_line = seq[0];
        range.start_col = seq[1];
        range.end_line = seq[2];
        range.end_col = seq[3];

        Ok(range)
    }
}

/// A set of annotations of a given type.
#[derive(Debug, Clone)]
pub struct Annotations {
    pub items: Spans<Value>,
    pub annotation_type: AnnotationType,
}

impl Annotations {
    /// Update the annotations in `interval` with the provided `items`.
    pub fn update(&mut self, interval: Interval, items: Spans<Value>) {
        self.items.edit(interval, items);
    }

    /// Remove annotations intersecting `interval`.
    pub fn invalidate(&mut self, interval: Interval) {
        self.items.delete_after(interval);
    }
}

/// A region of an `Annotation`.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AnnotationSlice {
    annotation_type: AnnotationType,
    /// Annotation occurrences, guaranteed non-descending start order.
    ranges: Vec<AnnotationRange>,
    /// If present, one payload per range.
    payloads: Option<Vec<Value>>,
}

impl AnnotationSlice {
    pub fn new(
        annotation_type: AnnotationType,
        ranges: Vec<AnnotationRange>,
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
    pub fn invalidate(&mut self, interval: Interval) {
        self.store.values_mut().flat_map(|v| v.iter_mut()).for_each(|a| a.invalidate(interval));
    }

    /// Applies an update from a plugin to a set of annotations
    pub fn update(&mut self, source: PluginId, interval: Interval, item: Annotations) {
        if !self.store.contains_key(&source) {
            self.store.insert(source, vec![item]);
            return;
        }

        let entry = self.store.get_mut(&source).unwrap();
        if let Some(annotation) =
            entry.iter_mut().find(|a| a.annotation_type == item.annotation_type)
        {
            annotation.update(interval, item.items);
        } else {
            entry.push(item);
        }
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
                // .filter() used instead of .subseq() because subseq() filters out spans with length 0
                let payloads = annotation
                    .items
                    .iter()
                    .filter(|(i, _p)| i.start() <= interval.end() && i.end() >= interval.start())
                    .map(|(_i, p)| p.clone())
                    .collect::<Vec<Value>>();

                let ranges = annotation
                    .items
                    .iter()
                    .filter(|(i, _p)| i.start() <= interval.end() && i.end() >= interval.start())
                    .map(|(i, _p)| {
                        let (start_line, start_col) = view.offset_to_line_col(text, i.start());
                        let (end_line, end_col) = view.offset_to_line_col(text, i.end());

                        AnnotationRange { start_line, start_col, end_line, end_col }
                    })
                    .collect::<Vec<AnnotationRange>>();

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::PluginPid;
    use crate::xi_rope::spans::SpansBuilder;

    #[test]
    fn test_annotation_range_serialization() {
        let range = AnnotationRange { start_line: 1, start_col: 3, end_line: 4, end_col: 1 };

        assert_eq!(json!(range).to_string(), "[1,3,4,1]")
    }

    #[test]
    fn test_annotation_range_deserialization() {
        let range: AnnotationRange = serde_json::from_str("[1,3,4,1]").unwrap();
        assert_eq!(range, AnnotationRange { start_line: 1, start_col: 3, end_line: 4, end_col: 1 })
    }

    #[test]
    fn test_annotation_slice_json() {
        let range = AnnotationRange { start_line: 1, start_col: 3, end_line: 4, end_col: 1 };

        let slice = AnnotationSlice {
            annotation_type: AnnotationType::Find,
            ranges: vec![range],
            payloads: None,
        };

        assert_eq!(
            slice.to_json().to_string(),
            "{\"n\":1,\"payloads\":null,\"ranges\":[[1,3,4,1]],\"type\":\"find\"}"
        )
    }

    #[test]
    fn test_annotation_store_update() {
        let mut store = AnnotationStore::new();

        let mut sb = SpansBuilder::new(10);
        sb.add_span(Interval::new(1, 5), json!(null));

        assert_eq!(store.store.len(), 0);

        store.update(
            PluginPid(1),
            Interval::new(1, 5),
            Annotations { annotation_type: AnnotationType::Find, items: sb.build() },
        );

        assert_eq!(store.store.len(), 1);

        sb = SpansBuilder::new(10);
        sb.add_span(Interval::new(6, 8), json!(null));

        store.update(
            PluginPid(2),
            Interval::new(6, 8),
            Annotations { annotation_type: AnnotationType::Find, items: sb.build() },
        );

        assert_eq!(store.store.len(), 2);
    }

    #[test]
    fn test_annotation_store_clear() {
        let mut store = AnnotationStore::new();

        let mut sb = SpansBuilder::new(10);
        sb.add_span(Interval::new(1, 5), json!(null));

        assert_eq!(store.store.len(), 0);

        store.update(
            PluginPid(1),
            Interval::new(1, 5),
            Annotations { annotation_type: AnnotationType::Find, items: sb.build() },
        );

        assert_eq!(store.store.len(), 1);

        sb = SpansBuilder::new(10);
        sb.add_span(Interval::new(6, 8), json!(null));

        store.update(
            PluginPid(2),
            Interval::new(6, 8),
            Annotations { annotation_type: AnnotationType::Find, items: sb.build() },
        );

        assert_eq!(store.store.len(), 2);

        store.clear(PluginPid(1));

        assert_eq!(store.store.len(), 1);

        store.clear(PluginPid(1));

        assert_eq!(store.store.len(), 1);
    }
}
