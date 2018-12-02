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

use std::collections::HashMap;
use serde_json::Value;

use plugins::PluginId;
use xi_rope::Interval;
use xi_rope::spans::Spans;

type AnnotationType = String;

/// Annotation types used in core.
pub enum CoreAnnotationType {
    Selection,
    Find
}

impl CoreAnnotationType {
    pub fn as_type(&self) -> AnnotationType {
        match self {
            CoreAnnotationType::Selection => "selection".to_string(),
            CoreAnnotationType::Find => "find".to_string(),
        }
    }
}

/// A set of annotations of a given type.
#[derive(Clone)]
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
    store: HashMap<PluginId, Vec<Annotations>>
}

impl AnnotationStore {
    pub fn new() -> Self {
        AnnotationStore {
            store: HashMap::new(),
        }
    }

    /// Applies an update from a plugin to a set of annotations
    fn update(&mut self, source: PluginId, type_id: AnnotationType, iv: Interval, items: Spans<Value>) {
        let updated_items = items.clone();
        let updated_type = type_id.clone();

        self.store.entry(source).and_modify(|e| {
            let outdated_annotations = e.iter().filter(|a|
                a.annotation_type == type_id
            ).cloned().collect::<Vec<Annotations>>();

            let mut annotations = e.iter().filter(|a|
                a.annotation_type != type_id
            ).cloned().collect::<Vec<Annotations>>();

            if !outdated_annotations.is_empty() {
                let mut updated_annotations = outdated_annotations.first().unwrap().clone();
                updated_annotations.update(iv, items);
                annotations.push(updated_annotations.clone());
            } else {
                annotations.push(Annotations {
                    items: items,
                    annotation_type: type_id
                });
            }

            *e = annotations;

        }).or_insert(vec![Annotations {
            items: updated_items,
            annotation_type: updated_type
        }]);
    }

    /// Returns an iterator which produces, for each type of annotation,
    /// those annotations which intersect the given interval.
    fn iter_range<'c>(&'c self, interval: Interval) -> impl Iterator<Item=AnnotationSlice> + 'c {
        let iv = interval.clone();
        self.store.iter().flat_map(move |(_plugin, value)| {
            value.iter().map(move |annotation| {
                let (ranges, payloads): (Vec<(usize, usize)>, Vec<Value>) = annotation.items.subseq(iv).iter().map(|(i, p)|
                    ((i.start(), i.end()), p.clone())
                ).unzip();

                AnnotationSlice {
                    annotation_type: annotation.annotation_type.clone(),
                    ranges: ranges,
                    payloads: Some(payloads)
                }
            })
        })
    }

    /// Removes any annotations provided by this plugin
    fn clear(&mut self, plugin: PluginId) {
        self.store.remove(&plugin);
    }
}