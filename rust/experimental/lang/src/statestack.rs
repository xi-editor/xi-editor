// Copyright 2017 The xi-editor Authors.
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

use std::{collections::HashMap, hash::Hash};

/// An entire state stack is represented as a single integer.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct State(usize);

struct Entry<T> {
    tos: T,
    prev: State,
}

/// All states are interpreted in a context.
pub struct Context<T> {
    // oddly enough, this is 1-based, as state 0 doesn't have an entry.
    entries: Vec<Entry<T>>,

    next: HashMap<(State, T), State>,
}

impl<T: Clone + Hash + Eq> Context<T> {
    pub fn new() -> Context<T> {
        Context { entries: Vec::new(), next: HashMap::new() }
    }

    fn entry(&self, s: State) -> Option<&Entry<T>> {
        if s.0 == 0 {
            None
        } else {
            Some(&self.entries[s.0 - 1])
        }
    }

    /// The top of the stack for the given state.
    pub fn tos(&self, s: State) -> Option<T> {
        self.entry(s).map(|entry| entry.tos.clone())
    }

    pub fn pop(&self, s: State) -> Option<State> {
        self.entry(s).map(|entry| entry.prev)
    }

    pub fn push(&mut self, s: State, el: T) -> State {
        let entries = &mut self.entries;
        *self.next.entry((s, el.clone())).or_insert_with(|| {
            entries.push(Entry { tos: el, prev: s });
            State(entries.len())
        })
    }
}
