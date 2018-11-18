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

use serde_json::{self, Value};


pub enum AnnotationType {
    Highlight,
    Selection
}

pub trait Annotation {
    fn annotation_type() -> AnnotationType;

    fn to_json(&self) -> Value;
}

pub struct SelectionAnnotation { }

impl Annotation for SelectionAnnotation {
    fn annotation_type() -> AnnotationType {
        return AnnotationType.Selection
    }

    fn to_json(&self) -> Value {
        json!({})   // todo
    }
}

pub struct HighlightAnnotation {
    pub query_id: usize
}

impl Annotation for HighlightAnnotation {
    fn annotation_type() -> AnnotationType {
        return AnnotationType.Highlight
    }

    fn to_json(&self) -> Value {
        json!({})   // todo
    }
}