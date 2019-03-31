// Copyright 2019 The xi-editor Authors.
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

use std::fmt;
use std::str::FromStr;

use serde::de::{self, Deserialize, Deserializer, Visitor};
use serde::ser::{Serialize, SerializeStruct, SerializeTupleVariant, Serializer};

use crate::tree::Node;
use crate::{Delta, DeltaElement, Rope, RopeInfo};

impl Serialize for Rope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&String::from(self))
    }
}

impl<'de> Deserialize<'de> for Rope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(RopeVisitor)
    }
}

struct RopeVisitor;

impl<'de> Visitor<'de> for RopeVisitor {
    type Value = Rope;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "a string")
    }

    fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Rope::from_str(s).map_err(|_| de::Error::invalid_value(de::Unexpected::Str(s), &self))
    }
}

impl Serialize for DeltaElement<RopeInfo> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match *self {
            DeltaElement::Copy(ref start, ref end) => {
                let mut el = serializer.serialize_tuple_variant("DeltaElement", 0, "copy", 2)?;
                el.serialize_field(start)?;
                el.serialize_field(end)?;
                el.end()
            }
            DeltaElement::Insert(ref node) => {
                serializer.serialize_newtype_variant("DeltaElement", 1, "insert", node)
            }
        }
    }
}

impl Serialize for Delta<RopeInfo> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut delta = serializer.serialize_struct("Delta", 2)?;
        delta.serialize_field("els", &self.els)?;
        delta.serialize_field("base_len", &self.base_len)?;
        delta.end()
    }
}

impl<'de> Deserialize<'de> for Delta<RopeInfo> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // NOTE: we derive to an interim representation and then convert
        // that into our actual target.
        #[derive(Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum RopeDeltaElement_ {
            Copy(usize, usize),
            Insert(Node<RopeInfo>),
        }

        #[derive(Serialize, Deserialize)]
        struct RopeDelta_ {
            els: Vec<RopeDeltaElement_>,
            base_len: usize,
        }

        impl From<RopeDeltaElement_> for DeltaElement<RopeInfo> {
            fn from(elem: RopeDeltaElement_) -> DeltaElement<RopeInfo> {
                match elem {
                    RopeDeltaElement_::Copy(start, end) => DeltaElement::Copy(start, end),
                    RopeDeltaElement_::Insert(rope) => DeltaElement::Insert(rope),
                }
            }
        }

        impl From<RopeDelta_> for Delta<RopeInfo> {
            fn from(mut delta: RopeDelta_) -> Delta<RopeInfo> {
                Delta {
                    els: delta.els.drain(..).map(DeltaElement::from).collect(),
                    base_len: delta.base_len,
                }
            }
        }
        let d = RopeDelta_::deserialize(deserializer)?;
        Ok(Delta::from(d))
    }
}
