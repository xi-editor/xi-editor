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
use std::iter;

use plugins::PluginId;
use view::View;
use xi_rope::spans::Spans;
use xi_rope::{Interval, Rope};

#[derive(Serialize, Deserialize, Debug, Clone)]
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
#[derive(Clone)]
pub struct Annotations {
    pub items: Spans<Value>,
    pub annotation_type: AnnotationType,
}

/// A region of an `Annotation`.
#[derive(Serialize, Deserialize, Debug)]
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
}

/// A trait for types (like `Selection`) that have a distinct representation
/// in core but are presented to the frontend as annotations.
pub trait ToAnnotation {
    /// Returns annotations that overlap the provided interval.
    fn get_annotations(&self, interval: Interval, view: &View, text: &Rope) -> AnnotationSlice;
}

/// All the annotations for a given view
pub struct AnnotationStore {
    _store: HashMap<PluginId, Vec<Annotations>>,
}

impl AnnotationStore {
    pub fn new() -> Self {
        AnnotationStore { _store: HashMap::new() }
    }

    /// Applies an update from a plugin to a set of annotations
    pub fn update(
        &mut self,
        _source: PluginId,
        _type_id: AnnotationType,
        _iv: Interval,
        _items: Spans<Value>,
    ) {
        // todo
    }

    /// Returns an iterator which produces, for each type of annotation,
    /// those annotations which intersect the given interval.
    pub fn iter_range<'c>(&'c self, _iv: Interval) -> impl Iterator<Item = AnnotationSlice> + 'c {
        // todo
        iter::empty()
    }

    /// Removes any annotations provided by this plugin
    pub fn clear(&mut self, _plugin: PluginId) {
        // todo
    }
}
