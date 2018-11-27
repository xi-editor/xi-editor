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

use std::{collections::HashMap, fmt::Debug, hash::Hash};

/// An entire state stack is represented as a single integer.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct State(usize);

impl State {
    pub fn raw(self) -> usize {
        self.0
    }
}

struct Entry<T> {
    tos: T,
    prev: State,
}

pub trait NewState<T> {
    fn new_state(&mut self, state: State, contents: &[T]);
    fn get_element(&self, state: State) -> &Option<T>;
}

/// All states are interpreted in a context.
pub struct Context<T, N> {
    new_state: N,

    // oddly enough, this is 1-based, as state 0 doesn't have an entry.
    entries: Vec<Entry<T>>,

    next: HashMap<(State, T), State>,
}

impl<T: Clone + Hash + Eq, N: NewState<T>> Context<T, N> {
    pub fn new(new_state: N) -> Context<T, N> {
        Context { new_state, entries: Vec::new(), next: HashMap::new() }
    }

    pub fn get_new_state(&self) -> &N {
        &self.new_state
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
        let mut new = false;
        let result = {
            let entries = &mut self.entries;
            *self.next.entry((s, el.clone())).or_insert_with(|| {
                new = true;
                entries.push(Entry { tos: el, prev: s });
                State(entries.len())
            })
        };
        if new {
            let contents = self.to_vec(result);
            self.new_state.new_state(result, &contents)
        }
        result
    }

    pub fn to_vec(&self, mut s: State) -> Vec<T> {
        let mut result = Vec::new();
        while let Some(entry) = self.entry(s) {
            result.push(entry.tos.clone());
            s = entry.prev;
        }
        result.reverse();
        result
    }
}

pub struct HolderNewState<T> {
    elements: Vec<Option<T>>,
}

impl<T> HolderNewState<T> {
    pub fn new() -> HolderNewState<T> {
        HolderNewState { elements: vec![None] }
    }
}

impl<T: Clone + Debug> NewState<T> for HolderNewState<T> {
    fn new_state(&mut self, _state: State, contents: &[T]) {
        for element in contents {
            self.elements.push(Some(element.clone()))
        }
    }

    fn get_element(&self, state: State) -> &Option<T> {
        &self.elements[state.raw()]
    }
}

pub struct DebugNewState;

impl DebugNewState {
    pub fn new() -> DebugNewState {
        DebugNewState
    }
}

impl<T: Debug> NewState<T> for DebugNewState {
    fn new_state(&mut self, state: State, contents: &[T]) {
        println!("new state {:?}: {:?}", state, contents);
    }

    fn get_element(&self, _state: State) -> &Option<T> {
        &None
    }
}
