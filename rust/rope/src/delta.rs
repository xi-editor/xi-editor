// Copyright 2016 Google Inc. All rights reserved.
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

//! A data structure for representing editing operations on ropes.
//! It's useful to explicitly represent these operations so they can be
//! shared across multiple subsystems.

use interval::Interval;
use tree::{Node, NodeInfo};
use std;

pub struct Delta<N: NodeInfo> {
    items: Vec<DeltaItem<N>>,
}

pub struct DeltaItem<N: NodeInfo> {
    pub interval: Interval,
    pub rope: Node<N>,
}

pub type Iter<'a, N> = std::slice::Iter<'a, DeltaItem<N>>;

impl<N: NodeInfo> Delta<N> {
    pub fn new() -> Delta<N> {
        Delta {
            items: Vec::new(),
        }
    }

    pub fn add(&mut self, interval: Interval, rope: Node<N>) {
        self.items.push(DeltaItem {
            interval: interval,
            rope: rope,
        })
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> Iter<N> {
        self.items.iter()
    }

    pub fn apply(&self, base: &mut Node<N>) {
        for item in self.iter() {
            base.edit(item.interval, item.rope.clone());
        }
    }
}
