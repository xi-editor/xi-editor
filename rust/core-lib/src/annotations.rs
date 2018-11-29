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

use plugins::PluginId;
use xi_rope::Interval;
use xi_rope::spans::Spans;

type AnnotationType = String;

/// Annotation types used in core.
pub enum Annotation {
    Selection,
    Find
}

impl Annotation {
    pub fn as_type(&self) -> AnnotationType {
        match self {
            Annotation::Selection => "selection".to_string(),
            Annotation::Find => "find".to_string(),
        }
    }
}

/// A set of annotations of a given type.
pub struct Annotations {
    items: Spans<Value>,
    annotation_type: AnnotationType,
}

/// A region of an `Annotation`.
#[derive(Serialize, Deserialize, Debug)]
pub struct AnnotationSlice {
    pub annotation_type: AnnotationType,
    /// Annotation occurrences, guaranteed non-descending start order.
    pub ranges: Vec<(usize, usize)>,
    /// If present, one payload per range.
    pub payloads: Option<Vec<Value>>,
}

/// A trait for types (like `Selection`) that have a distinct representation
/// in core but are presented to the frontend as annotations.
pub trait ToAnnotation {
    /// Returns annotations that overlap the provided interval.
    fn get_annotations(&self, interval: Interval) -> AnnotationSlice;
}

/// All the annotations for a given view
pub struct AnnotationStore {
    //store: HashMap<PluginId, HashMap<AnnotationType, Annotations>>
    store: Vec<Annotations>
}

impl AnnotationStore {
    pub fn new() -> Self {
        AnnotationStore {
            store: Vec::new(),
        }
    }

    /// Applies an update from a plugin to a set of annotations
//    fn update(&mut self, source: PluginId, type_id: String, range: Range, items: Vec<_>) { }
    /// Returns an iterator which produces, for each type of annotation,
    /// those annotations which intersect the given interval.
//    fn iter_range(&self, interval: Interval) -> impl Iterator<Item=AnnotationSlice> { }
    /// Removes any annotations provided by this plugin
    fn clear(&mut self, plugin: PluginId) { }
}