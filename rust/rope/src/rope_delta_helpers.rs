// Copyright 2018 Google LLC
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

//! Helpers for working with text `Delta`s.

use std::iter::{once, FromIterator};
use std::slice;

use delta::{Builder, Delta, DeltaElement};
use interval::Interval;
use rope::{Rope, RopeInfo};

/// A modification to a contiguious region of a text document.
///
/// A `Delta` can be converted to a series of `Edit`s, which may be more
/// useful for certain kinds of operations.
///
/// Specifically, converting a `Delta` into a series of `Edit`s provides a
/// more ergonomic way of iterating over a set of changes in a document.
///
/// This is useful for things like xi plugins, where it is common for a plugin
/// author to want a simple high-level view of what regions of a document
/// have changed in a given revision.
#[derive(Debug, Clone, Default)]
pub struct Edit {
    /// The start offset of this edit.
    pub start: usize,
    /// The end offset of this edit.
    pub end: usize,
    /// If present, text to insert in place of (start, end].
    pub contents: Option<Rope>,
    base_len: usize,
}

pub struct Iter<'a> {
    last_end: usize,
    base_len: usize,
    els_iter: slice::Iter<'a, DeltaElement<RopeInfo>>,
}

impl Edit {
    /// Convenience constructor used in the iterator.
    fn new_partial(start: usize, base_len: usize) -> Self {
        Edit {
            start,
            end: base_len,
            contents: None,
            base_len,
        }
    }
}

impl Delta<RopeInfo> {
    /// Returns an iterator over the `Edit`s that constitute this `Delta`.
    pub fn iter_edits(&self) -> Iter {
        Iter {
            last_end: 0,
            base_len: self.base_len,
            els_iter: self.els.iter(),
        }
    }

    /// If this delta can be represented as a single `Edit`, returns that edit.
    pub fn as_single_edit(&self) -> Option<Edit> {
        let mut iter = self.iter_edits();
        let first = iter.next();
        let second = iter.next();
        if second.is_some() {
            None
        } else {
            first
        }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = Edit;

    fn next(&mut self) -> Option<Self::Item> {
        let mut result: Option<Edit> = None;
        while let Some(elem) = self.els_iter.next() {
            match *elem {
                DeltaElement::Copy(b, e) => {
                    if let Some(mut r) = result {
                        r.end = b;
                        self.last_end = e;
                        return Some(r);
                    } else if b > self.last_end {
                        let start = self.last_end;
                        self.last_end = e;
                        return Some(Edit {
                            start,
                            end: b,
                            contents: None,
                            base_len: self.base_len,
                        });
                    } else {
                        result = Some(Edit::new_partial(e, self.base_len));
                        self.last_end = e;
                    }
                }
                DeltaElement::Insert(ref n) => {
                    if let Some(mut r) = result.as_mut() {
                        r.contents = Some(n.clone());
                        continue;
                    }

                    let mut nxt = Edit::new_partial(self.last_end, self.base_len);
                    nxt.contents = Some(n.clone());
                    result = Some(nxt);
                }
            }
        }

        if result.is_none() && self.last_end != self.base_len {
            result = Some(Edit::new_partial(self.last_end, self.base_len));
        }

        let should_return_last = result.as_ref()
            .map(|r| r.contents.is_some() || r.start != r.end)
            .unwrap_or(false);

        if should_return_last
        && result.as_ref().map(|r| r.end == self.base_len).unwrap() {
            // if last item is an insert, no need for an extra delete
            self.last_end = self.base_len;

        }

        if should_return_last {
            result
        } else {
            None
        }
    }
}

impl FromIterator<Edit> for Delta<RopeInfo> {
    fn from_iter<T: IntoIterator<Item = Edit>>(iter: T) -> Self {
        let mut iter = iter.into_iter();

        let first = iter.next();
        let base_len = first.as_ref().map(|edit| edit.base_len).unwrap_or(0);
        let mut builder = Builder::new(base_len);

        if first.is_none() {
            return builder.build();
        }

        for edit in once(first.unwrap()).chain(iter) {
            assert_eq!(edit.base_len, base_len, "grouped edits must share base_len");
            let iv = Interval::new_closed_open(edit.start, edit.end);
            if let Some(text) = edit.contents {
                builder.replace(iv, text);
            } else {
                builder.delete(iv);
            }
        }
        builder.build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_test() {
        let base = "one two tri";
        let delta = Delta::simple_edit(Interval::new_open_closed(4, 7), "blu".into(), base.len());
        assert_eq!("one blu tri", delta.apply_to_string(base));

        let edit = delta.as_single_edit().unwrap();
        assert_eq!(edit.start, 4, "{:?}", edit);
        assert_eq!(edit.end, 7);
        assert_eq!(edit.contents, Some("blu".into()));
    }

    #[test]
    fn multi_edit_delta() {
        let base = "one two tri";
        let mut builder = Builder::new(base.len());
        builder.replace(Interval::new_closed_open(4, 7), "blu".into());
        builder.replace(Interval::new_closed_open(10, 11), "ee".into());
        let delta = builder.build();

        assert_eq!("one blu tree", delta.apply_to_string(base));
        assert!(delta.as_single_edit().is_none());

        assert_eq!(delta.els.len(), 4);
        let mut iter = delta.iter_edits();
        let first = iter.next().unwrap();

        assert_eq!(first.start, 4, "{:?}", first);
        assert_eq!(first.end, 7);
        assert_eq!(first.contents, Some("blu".into()));

        let second = iter.next().unwrap();

        assert_eq!(second.start, 10, "{:?}", second);
        assert_eq!(second.end, 11);
        assert_eq!(second.contents, Some("ee".into()));

        let delta_again: Delta<RopeInfo> = delta.iter_edits().collect();
        assert_eq!(delta.els.len(), delta_again.els.len());
        assert_eq!(delta.apply_to_string(base), delta_again.apply_to_string(base));
    }


    #[test]
    fn delete_start_and_end() {
        let base = "one two tri";
        let mut builder = Builder::new(base.len());
        builder.delete(Interval::new_closed_open(0, 4));
        builder.replace(Interval::new_closed_open(5, 6), "h".into());
        builder.delete(Interval::new_closed_open(7, 11));
        let delta = builder.build();

        assert_eq!("tho", delta.apply_to_string(base));

        let mut iter = delta.iter_edits();

        let first = iter.next().expect("first");

        assert_eq!(first.start, 0);
        assert_eq!(first.end, 4);
        assert!(first.contents.is_none());

        let second = iter.next().expect("second");

        assert_eq!(second.start, 5, "{:?}", second);
        assert_eq!(second.end, 6);
        assert_eq!(second.contents, Some("h".into()));

        let third = iter.next().expect("third");

        assert_eq!(third.start, 7, "{:?}", third);
        assert_eq!(third.end, 11);
        assert!(third.contents.is_none());

        let delta_again: Delta<RopeInfo> = delta.iter_edits().collect();
        assert_eq!(delta.els.len(), delta_again.els.len());
        assert_eq!(delta.apply_to_string(base), delta_again.apply_to_string(base));
    }
}
