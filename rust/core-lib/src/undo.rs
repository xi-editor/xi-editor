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

//! Utilities for tracking undo state. This does not currently handle
//! the main undo stack, which is still part of `Editor`. Maybe one day?

use std::collections::VecDeque;
use std::fmt::{Debug, Display};

type UndoGroup = usize;

/// Tracking auxillary state for the purposes of undo.
pub(crate) struct ViewUndoStack<T> {
    groups: VecDeque<ViewUndo<T>>,
    /// The index into `groups` of the currently active group.
    cur_idx: usize,
    capacity: usize,
}

/// Stores additional state associated with a particular undo group.
struct ViewUndo<T> {
    group: UndoGroup,
    /// The state to be restored when this group is undone. This is `None`
    /// if it would be equal to the previous group's `after` state.
    before: Option<T>,
    /// The state to be restored when this group is redone.
    after: T,
}

impl<T: Clone + PartialEq + Display + Debug> ViewUndoStack<T> {
    pub(crate) fn new(capacity: usize, initial: T) -> Self {
        let initial = ViewUndo { group: 0, before: None, after: initial };
        let mut groups = VecDeque::new();
        groups.push_back(initial);
        let cur_idx = 0;
        ViewUndoStack { groups, cur_idx, capacity }
    }

    pub(crate) fn active_undo_group(&self) -> UndoGroup {
        self.groups[self.cur_idx].group
    }

    fn max_group(&self) -> UndoGroup {
        assert!(!self.groups.is_empty());
        self.groups.back().unwrap().group
    }

    /// Adds a new undo group. Any currently undone groups will be dropped.
    pub(crate) fn new_group(&mut self, group: UndoGroup, before: &T, after: T) {
        debug_assert!(group > self.max_group());
        if self.cur_idx < self.groups.len() - 1 {
            self.groups.truncate(self.cur_idx + 1);
        }
        let needs_before = before != &self.groups[self.cur_idx].after;
        let before = if needs_before { Some(before.to_owned()) } else { None };

        self.groups.push_back(ViewUndo { group, before, after });
        if self.groups.len() > self.capacity {
            debug_assert_eq!(self.cur_idx, self.capacity - 1);
            self.groups.pop_front();
        } else {
            self.cur_idx += 1;
        }
        debug_assert_eq!(self.cur_idx, self.groups.len() - 1);
    }

    /// Updates the 'after' state for a given undo group. This should be called
    /// when an edit occurs that modifies an existing group.
    pub(crate) fn update_group(&mut self, group: UndoGroup, after: T) {
        debug_assert!(self.groups[self.cur_idx].group == group);
        self.groups[self.cur_idx].after = after;
    }

    /// Return the state saved before the current undo group, and select
    ///
    /// # Panics
    ///
    /// This should only be called when a undo has been successfully
    /// applied in the editor. This function will panic if there is no group
    /// to undo.
    pub(crate) fn undo(&mut self) -> &T {
        assert!(self.cur_idx > 0, "UndoState::undo called with no undo stack.");
        self.cur_idx -= 1;
        self.groups[self.cur_idx + 1].before.as_ref().unwrap_or(&self.groups[self.cur_idx].after)
    }

    /// Select the next undo group, returning the associated state saved after
    /// that group's last edit.
    ///
    /// # Panics
    ///
    /// This should only be called when a redo has been successfully
    /// applied in the editor. This function will panic if there is no group
    /// to redo.
    pub(crate) fn redo(&mut self) -> &T {
        self.cur_idx += 1;
        assert!(self.cur_idx < self.groups.len(), "UndoState::redo called with no items to redo");
        &self.groups[self.cur_idx].after
    }

    #[allow(dead_code)]
    fn debug_print(&self, prelude: &str) {
        let current = &self.groups[self.cur_idx];
        eprintln!(
            "{}: {} ({}/{}), {}{}",
            prelude,
            current.group,
            self.cur_idx + 1,
            self.groups.len(),
            current.before.as_ref().map(|s| format!("{} <-> ", s)).unwrap_or(String::new()),
            current.after
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "no undo stack")]
    fn capacity() {
        let capacity = 5;
        let mut undo = ViewUndoStack::new(capacity, 'a');
        undo.new_group(1, &'b', 'B');
        undo.new_group(2, &'c', 'C');
        undo.new_group(3, &'d', 'D');
        undo.new_group(4, &'e', 'E');
        undo.new_group(5, &'f', 'F');
        undo.new_group(6, &'g', 'G');
        assert_eq!(*undo.undo(), 'g');
        undo.undo();
        undo.undo();
        let x = *undo.undo();
        assert_eq!(x, 'd');
        // now crash, end of stack
        undo.undo();
    }

    #[test]
    #[should_panic(expected = "no items to redo")]
    fn undo_redo() {
        let capacity = 5;
        let mut undo = ViewUndoStack::new(capacity, 'a');
        undo.new_group(1, &'b', 'B');
        undo.new_group(2, &'c', 'C');
        undo.update_group(2, 'Ç');
        assert_eq!(*undo.undo(), 'c');
        assert_eq!(*undo.undo(), 'b');
        assert_eq!(*undo.redo(), 'B');
        assert_eq!(*undo.redo(), 'Ç');

        // now crash, end of stack
        undo.redo();
    }
}
