// Copyright 2016 The xi-editor Authors.
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
#![warn(rust_2018_idioms)]
//! Trees for text.

#[cfg(feature = "serde")]
#[macro_use]
extern crate serde;

pub mod breaks;
pub mod compare;
pub mod delta;
pub mod diff;
pub mod engine;
pub mod find;
pub mod interval;
pub mod multiset;
pub mod rope;
#[cfg(feature = "serde")]
mod serde_impls;
pub mod spans;
#[cfg(test)]
mod test_helpers;
pub mod tree;

pub use crate::delta::{Delta, DeltaBuilder, DeltaElement, Transformer};
pub use crate::interval::Interval;
pub use crate::rope::{LinesMetric, Rope, RopeDelta, RopeInfo};
pub use crate::tree::{Cursor, Metric};
