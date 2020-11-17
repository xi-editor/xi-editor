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

//! StackMap is a simple nested map type (a trie) used to map `StackScope`s to
//! u32s so they can be efficiently sent to xi-core.
//!
//! For discussion of this approach, see [this
//! issue](https://github.com/google/xi-editor/issues/284).
use std::collections::HashMap;

use syntect::parsing::Scope;

#[derive(Debug, Default)]
struct Node {
    value: Option<u32>,
    children: HashMap<Scope, Node>,
}

#[derive(Debug, Default)]
/// Nested lookup table for stacks of scopes.
pub struct StackMap {
    next_id: u32,
    scopes: Node,
}

#[derive(Debug, PartialEq)]
/// Result type for `StackMap` lookups. Used to communicate to the user
/// whether or not a new identifier has been assigned, which will need to
/// be communicated to the peer.
pub enum LookupResult {
    Existing(u32),
    New(u32),
}

impl Node {
    pub fn new(value: u32) -> Self {
        Node { value: Some(value), children: HashMap::new() }
    }

    fn get_value(&mut self, stack: &[Scope], next_id: u32) -> LookupResult {
        // if this is last item on the stack, get the value, inserting if necessary.
        let first = stack.first().unwrap();
        if stack.len() == 1 {
            if !self.children.contains_key(first) {
                self.children.insert(first.to_owned(), Node::new(next_id));
                return LookupResult::New(next_id);
            }

            // if key exists, value still might not be assigned:
            let needs_value = self.children[first].value.is_none();
            if needs_value {
                let node = self.children.get_mut(first).unwrap();
                node.value = Some(next_id);
                return LookupResult::New(next_id);
            } else {
                let value = self.children[first].value.unwrap();
                return LookupResult::Existing(value);
            }
        }
        // not the last item: recurse, creating node as necessary
        if self.children.get(first).is_none() {
            self.children.insert(first.to_owned(), Node::default());
        }
        self.children.get_mut(first).unwrap().get_value(&stack[1..], next_id)
    }
}

impl StackMap {
    /// Returns the identifier for this stack, creating it if needed.
    pub fn get_value(&mut self, stack: &[Scope]) -> LookupResult {
        assert!(!stack.is_empty());
        let result = self.scopes.get_value(stack, self.next_id);
        if result.is_new() {
            self.next_id += 1;
        }
        result
    }
}

impl LookupResult {
    pub fn is_new(&self) -> bool {
        matches!(*self, LookupResult::New(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use syntect::parsing::ScopeStack;

    #[test]
    fn test_get_value() {
        let mut stackmap = StackMap::default();
        let stack = ScopeStack::from_str("text.rust.test scope.level.three").unwrap();
        assert_eq!(stack.as_slice().len(), 2);
        assert_eq!(stackmap.get_value(stack.as_slice()), LookupResult::New(0));
        assert_eq!(stackmap.get_value(stack.as_slice()), LookupResult::Existing(0));
        // we don't assign values to intermediate scopes during traversal
        let stack2 = ScopeStack::from_str("text.rust.test").unwrap();
        assert_eq!(stackmap.get_value(stack2.as_slice()), LookupResult::New(1));
        assert_eq!(stack2.as_slice().len(), 1);
    }
}
