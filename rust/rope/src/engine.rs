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

//! An engine for handling edits (possibly from async sources) and undo. It
//! conceptually represents the current text and all edit history for that
//! text.
//!
//! This module actually implements a mini Conflict-free Replicated Data Type
//! under `Engine::edit_rev`, which is considerably simpler than the usual
//! CRDT implementation techniques, because all operations are serialized in
//! this central engine. It provides the ability to apply edits that depend on
//! a previously committed version of the text rather than the current text,
//! which is sufficient for asynchronous plugins that can only have one
//! pending edit in flight each.
//!
//! There is also a full CRDT merge operation implemented under
//! `Engine::merge`, which is more powerful but considerably more complex.
//! It enables support for full asynchronous and even peer-to-peer editing.

use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeSet;

use crate::delta::{Delta, InsertDelta};
use crate::interval::Interval;
use crate::multiset::{CountMatcher, Subset};
use crate::rope::{Rope, RopeInfo};

/// Represents the current state of a document and all of its history
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Engine {
    /// The session ID used to create new `RevId`s for edits made on this device
    #[cfg_attr(feature = "serde", serde(default = "default_session", skip_serializing))]
    session: SessionId,
    /// The incrementing revision number counter for this session used for `RevId`s
    #[cfg_attr(feature = "serde", serde(default = "initial_revision_counter", skip_serializing))]
    rev_id_counter: u32,
    /// The current contents of the document as would be displayed on screen
    text: Rope,
    /// Storage for all the characters that have been deleted  but could
    /// return if a delete is un-done or an insert is re- done.
    tombstones: Rope,
    /// Imagine a "union string" that contained all the characters ever
    /// inserted, including the ones that were later deleted, in the locations
    /// they would be if they hadn't been deleted.
    ///
    /// This is a `Subset` of the "union string" representing the characters
    /// that are currently deleted, and thus in `tombstones` rather than
    /// `text`. The count of a character in `deletes_from_union` represents
    /// how many times it has been deleted, so if a character is deleted twice
    /// concurrently it will have count `2` so that undoing one delete but not
    /// the other doesn't make it re-appear.
    ///
    /// You could construct the "union string" from `text`, `tombstones` and
    /// `deletes_from_union` by splicing a segment of `tombstones` into `text`
    /// wherever there's a non-zero-count segment in `deletes_from_union`.
    deletes_from_union: Subset,
    // TODO: switch to a persistent Set representation to avoid O(n) copying
    undone_groups: BTreeSet<usize>, // set of undo_group id's
    /// The revision history of the document
    revs: Vec<Revision>,
}

// The advantage of using a session ID over random numbers is that it can be
// easily delta-compressed later.
#[derive(Debug, Clone, Copy, PartialOrd, Ord, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct RevId {
    // 96 bits has a 10^(-12) chance of collision with 400 million sessions and 10^(-6) with 100 billion.
    // `session1==session2==0` is reserved for initialization which is the same on all sessions.
    // A colliding session will break merge invariants and the document will start crashing Xi.
    session1: u64,
    // if this was a tuple field instead of two fields, alignment padding would add 8 more bytes.
    session2: u32,
    // There will probably never be a document with more than 4 billion edits
    // in a single session.
    num: u32,
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
struct Revision {
    /// This uniquely represents the identity of this revision and it stays
    /// the same even if it is rebased or merged between devices.
    rev_id: RevId,
    /// The largest undo group number of any edit in the history up to this
    /// point. Used to optimize undo to not look further back.
    max_undo_so_far: usize,
    edit: Contents,
}

/// Valid within a session. If there's a collision the most recent matching
/// Revision will be used, which means only the (small) set of concurrent edits
/// could trigger incorrect behavior if they collide, so u64 is safe.
pub type RevToken = u64;

/// the session ID component of a `RevId`
pub type SessionId = (u64, u32);

/// Type for errors that occur during CRDT operations.
#[derive(Clone)]
pub enum Error {
    /// An edit specified a revision that did not exist. The revision may
    /// have been GC'd, or it may have specified incorrectly.
    MissingRevision(RevToken),
    /// A delta was applied which had a `base_len` that did not match the length
    /// of the revision it was applied to.
    MalformedDelta { rev_len: usize, delta_len: usize },
}

#[derive(Clone, Copy, PartialOrd, Ord, PartialEq, Eq)]
struct FullPriority {
    priority: usize,
    session_id: SessionId,
}

use self::Contents::*;

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
enum Contents {
    Edit {
        /// Used to order concurrent inserts, for example auto-indentation
        /// should go before typed text.
        priority: usize,
        /// Groups related edits together so that they are undone and re-done
        /// together. For example, an auto-indent insertion would be un-done
        /// along with the newline that triggered it.
        undo_group: usize,
        /// The subset of the characters of the union string from after this
        /// revision that were added by this revision.
        inserts: Subset,
        /// The subset of the characters of the union string from after this
        /// revision that were deleted by this revision.
        deletes: Subset,
    },
    Undo {
        /// The set of groups toggled between undone and done.
        /// Just the `symmetric_difference` (XOR) of the two sets.
        toggled_groups: BTreeSet<usize>, // set of undo_group id's
        /// Used to store a reversible difference between the old
        /// and new deletes_from_union
        deletes_bitxor: Subset,
    },
}

/// for single user cases, used by serde and ::empty
fn default_session() -> (u64, u32) {
    (1, 0)
}

/// Revision 0 is always an Undo of the empty set of groups
#[cfg(feature = "serde")]
fn initial_revision_counter() -> u32 {
    1
}

impl RevId {
    /// Returns a u64 that will be equal for equivalent revision IDs and
    /// should be as unlikely to collide as two random u64s.
    pub fn token(&self) -> RevToken {
        use std::hash::{Hash, Hasher};
        // Rust is unlikely to break the property that this hash is strongly collision-resistant
        // and it only needs to be consistent over one execution.
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }

    pub fn session_id(&self) -> SessionId {
        (self.session1, self.session2)
    }
}

impl Engine {
    /// Create a new Engine with a single edit that inserts `initial_contents`
    /// if it is non-empty. It needs to be a separate commit rather than just
    /// part of the initial contents since any two `Engine`s need a common
    /// ancestor in order to be mergeable.
    pub fn new(initial_contents: Rope) -> Engine {
        let mut engine = Engine::empty();
        if !initial_contents.is_empty() {
            let first_rev = engine.get_head_rev_id().token();
            let delta = Delta::simple_edit(Interval::new(0, 0), initial_contents, 0);
            engine.edit_rev(0, 0, first_rev, delta);
        }
        engine
    }

    pub fn empty() -> Engine {
        let deletes_from_union = Subset::new(0);
        let rev = Revision {
            rev_id: RevId { session1: 0, session2: 0, num: 0 },
            edit: Undo {
                toggled_groups: BTreeSet::new(),
                deletes_bitxor: deletes_from_union.clone(),
            },
            max_undo_so_far: 0,
        };
        Engine {
            session: default_session(),
            rev_id_counter: 1,
            text: Rope::default(),
            tombstones: Rope::default(),
            deletes_from_union,
            undone_groups: BTreeSet::new(),
            revs: vec![rev],
        }
    }

    fn next_rev_id(&self) -> RevId {
        RevId { session1: self.session.0, session2: self.session.1, num: self.rev_id_counter }
    }

    fn find_rev(&self, rev_id: RevId) -> Option<usize> {
        self.revs
            .iter()
            .enumerate()
            .rev()
            .find(|&(_, ref rev)| rev.rev_id == rev_id)
            .map(|(i, _)| i)
    }

    fn find_rev_token(&self, rev_token: RevToken) -> Option<usize> {
        self.revs
            .iter()
            .enumerate()
            .rev()
            .find(|&(_, ref rev)| rev.rev_id.token() == rev_token)
            .map(|(i, _)| i)
    }

    // TODO: does Cow really help much here? It certainly won't after making Subsets a rope.
    /// Find what the `deletes_from_union` field in Engine would have been at the time
    /// of a certain `rev_index`. In other words, the deletes from the union string at that time.
    fn deletes_from_union_for_index(&self, rev_index: usize) -> Cow<Subset> {
        self.deletes_from_union_before_index(rev_index + 1, true)
    }

    /// Garbage collection means undo can sometimes need to replay the very first
    /// revision, and so needs a way to get the deletion set before then.
    fn deletes_from_union_before_index(&self, rev_index: usize, invert_undos: bool) -> Cow<Subset> {
        let mut deletes_from_union = Cow::Borrowed(&self.deletes_from_union);
        let mut undone_groups = Cow::Borrowed(&self.undone_groups);

        // invert the changes to deletes_from_union starting in the present and working backwards
        for rev in self.revs[rev_index..].iter().rev() {
            deletes_from_union = match rev.edit {
                Edit { ref inserts, ref deletes, ref undo_group, .. } => {
                    if undone_groups.contains(undo_group) {
                        // no need to un-delete undone inserts since we'll just shrink them out
                        Cow::Owned(deletes_from_union.transform_shrink(inserts))
                    } else {
                        let un_deleted = deletes_from_union.subtract(deletes);
                        Cow::Owned(un_deleted.transform_shrink(inserts))
                    }
                }
                Undo { ref toggled_groups, ref deletes_bitxor } => {
                    if invert_undos {
                        let new_undone =
                            undone_groups.symmetric_difference(toggled_groups).cloned().collect();
                        undone_groups = Cow::Owned(new_undone);
                        Cow::Owned(deletes_from_union.bitxor(deletes_bitxor))
                    } else {
                        deletes_from_union
                    }
                }
            }
        }
        deletes_from_union
    }

    /// Get the contents of the document at a given revision number
    fn rev_content_for_index(&self, rev_index: usize) -> Rope {
        let old_deletes_from_union = self.deletes_from_cur_union_for_index(rev_index);
        let delta =
            Delta::synthesize(&self.tombstones, &self.deletes_from_union, &old_deletes_from_union);
        delta.apply(&self.text)
    }

    /// Get the Subset to delete from the current union string in order to obtain a revision's content
    fn deletes_from_cur_union_for_index(&self, rev_index: usize) -> Cow<Subset> {
        let mut deletes_from_union = self.deletes_from_union_for_index(rev_index);
        for rev in &self.revs[rev_index + 1..] {
            if let Edit { ref inserts, .. } = rev.edit {
                if !inserts.is_empty() {
                    deletes_from_union = Cow::Owned(deletes_from_union.transform_union(inserts));
                }
            }
        }
        deletes_from_union
    }

    /// Returns the largest undo group ID used so far
    pub fn max_undo_group_id(&self) -> usize {
        self.revs.last().unwrap().max_undo_so_far
    }

    /// Get revision id of head revision.
    pub fn get_head_rev_id(&self) -> RevId {
        self.revs.last().unwrap().rev_id
    }

    /// Get text of head revision.
    pub fn get_head(&self) -> &Rope {
        &self.text
    }

    /// Get text of a given revision, if it can be found.
    pub fn get_rev(&self, rev: RevToken) -> Option<Rope> {
        self.find_rev_token(rev).map(|rev_index| self.rev_content_for_index(rev_index))
    }

    /// A delta that, when applied to `base_rev`, results in the current head. Returns
    /// an error if there is not at least one edit.
    pub fn try_delta_rev_head(&self, base_rev: RevToken) -> Result<Delta<RopeInfo>, Error> {
        let ix = self.find_rev_token(base_rev).ok_or_else(|| Error::MissingRevision(base_rev))?;
        let prev_from_union = self.deletes_from_cur_union_for_index(ix);
        // TODO: this does 2 calls to Delta::synthesize and 1 to apply, this probably could be better.
        let old_tombstones = shuffle_tombstones(
            &self.text,
            &self.tombstones,
            &self.deletes_from_union,
            &prev_from_union,
        );
        Ok(Delta::synthesize(&old_tombstones, &prev_from_union, &self.deletes_from_union))
    }

    // TODO: don't construct transform if subsets are empty
    // TODO: maybe switch to using a revision index for `base_rev` once we disable GC
    /// Returns a tuple of a new `Revision` representing the edit based on the
    /// current head, a new text `Rope`, a new tombstones `Rope` and a new `deletes_from_union`.
    /// Returns an [`Error`] if `base_rev` cannot be found, or `delta.base_len`
    /// does not equal the length of the text at `base_rev`.
    fn mk_new_rev(
        &self,
        new_priority: usize,
        undo_group: usize,
        base_rev: RevToken,
        delta: Delta<RopeInfo>,
    ) -> Result<(Revision, Rope, Rope, Subset), Error> {
        let ix = self.find_rev_token(base_rev).ok_or_else(|| Error::MissingRevision(base_rev))?;

        let (ins_delta, deletes) = delta.factor();

        // rebase delta to be on the base_rev union instead of the text
        let deletes_at_rev = self.deletes_from_union_for_index(ix);

        // validate delta
        if ins_delta.base_len != deletes_at_rev.len_after_delete() {
            return Err(Error::MalformedDelta {
                delta_len: ins_delta.base_len,
                rev_len: deletes_at_rev.len_after_delete(),
            });
        }

        let mut union_ins_delta = ins_delta.transform_expand(&deletes_at_rev, true);
        let mut new_deletes = deletes.transform_expand(&deletes_at_rev);

        // rebase the delta to be on the head union instead of the base_rev union
        let new_full_priority = FullPriority { priority: new_priority, session_id: self.session };
        for r in &self.revs[ix + 1..] {
            if let Edit { priority, ref inserts, .. } = r.edit {
                if !inserts.is_empty() {
                    let full_priority =
                        FullPriority { priority, session_id: r.rev_id.session_id() };
                    let after = new_full_priority >= full_priority; // should never be ==
                    union_ins_delta = union_ins_delta.transform_expand(inserts, after);
                    new_deletes = new_deletes.transform_expand(inserts);
                }
            }
        }

        // rebase the deletion to be after the inserts instead of directly on the head union
        let new_inserts = union_ins_delta.inserted_subset();
        if !new_inserts.is_empty() {
            new_deletes = new_deletes.transform_expand(&new_inserts);
        }

        // rebase insertions on text and apply
        let cur_deletes_from_union = &self.deletes_from_union;
        let text_ins_delta = union_ins_delta.transform_shrink(cur_deletes_from_union);
        let text_with_inserts = text_ins_delta.apply(&self.text);
        let rebased_deletes_from_union = cur_deletes_from_union.transform_expand(&new_inserts);

        // is the new edit in an undo group that was already undone due to concurrency?
        let undone = self.undone_groups.contains(&undo_group);
        let new_deletes_from_union = {
            let to_delete = if undone { &new_inserts } else { &new_deletes };
            rebased_deletes_from_union.union(to_delete)
        };

        // move deleted or undone-inserted things from text to tombstones
        let (new_text, new_tombstones) = shuffle(
            &text_with_inserts,
            &self.tombstones,
            &rebased_deletes_from_union,
            &new_deletes_from_union,
        );

        let head_rev = &self.revs.last().unwrap();
        Ok((
            Revision {
                rev_id: self.next_rev_id(),
                max_undo_so_far: std::cmp::max(undo_group, head_rev.max_undo_so_far),
                edit: Edit {
                    priority: new_priority,
                    undo_group,
                    inserts: new_inserts,
                    deletes: new_deletes,
                },
            },
            new_text,
            new_tombstones,
            new_deletes_from_union,
        ))
    }
    // NOTE: maybe just deprecate this? we can panic on the other side of
    // the call if/when that makes sense.
    /// Create a new edit based on `base_rev`.
    ///
    /// # Panics
    ///
    /// Panics if `base_rev` does not exist, or if `delta` is poorly formed.
    pub fn edit_rev(
        &mut self,
        priority: usize,
        undo_group: usize,
        base_rev: RevToken,
        delta: Delta<RopeInfo>,
    ) {
        self.try_edit_rev(priority, undo_group, base_rev, delta).unwrap();
    }

    // TODO: have `base_rev` be an index so that it can be used maximally
    // efficiently with the head revision, a token or a revision ID.
    // Efficiency loss of token is negligible but unfortunate.
    /// Attempts to apply a new edit based on the [`Revision`] specified by `base_rev`,
    /// Returning an [`Error`] if the `Revision` cannot be found.
    pub fn try_edit_rev(
        &mut self,
        priority: usize,
        undo_group: usize,
        base_rev: RevToken,
        delta: Delta<RopeInfo>,
    ) -> Result<(), Error> {
        let (new_rev, new_text, new_tombstones, new_deletes_from_union) =
            self.mk_new_rev(priority, undo_group, base_rev, delta)?;
        self.rev_id_counter += 1;
        self.revs.push(new_rev);
        self.text = new_text;
        self.tombstones = new_tombstones;
        self.deletes_from_union = new_deletes_from_union;
        Ok(())
    }

    // since undo and gc replay history with transforms, we need an empty set
    // of the union string length *before* the first revision.
    fn empty_subset_before_first_rev(&self) -> Subset {
        let first_rev = &self.revs.first().unwrap();
        // it will be immediately transform_expanded by inserts if it is an Edit, so length must be before
        let len = match first_rev.edit {
            Edit { ref inserts, .. } => inserts.count(CountMatcher::Zero),
            Undo { ref deletes_bitxor, .. } => deletes_bitxor.count(CountMatcher::All),
        };
        Subset::new(len)
    }

    /// Find the first revision that could be affected by toggling a set of undo groups
    fn find_first_undo_candidate_index(&self, toggled_groups: &BTreeSet<usize>) -> usize {
        // find the lowest toggled undo group number
        if let Some(lowest_group) = toggled_groups.iter().cloned().next() {
            for (i, rev) in self.revs.iter().enumerate().rev() {
                if rev.max_undo_so_far < lowest_group {
                    return i + 1; // +1 since we know the one we just found doesn't have it
                }
            }
            0
        } else {
            // no toggled groups, return past end
            self.revs.len()
        }
    }

    // This computes undo all the way from the beginning. An optimization would be to not
    // recompute the prefix up to where the history diverges, but it's not clear that's
    // even worth the code complexity.
    fn compute_undo(&self, groups: &BTreeSet<usize>) -> (Revision, Subset) {
        let toggled_groups = self.undone_groups.symmetric_difference(&groups).cloned().collect();
        let first_candidate = self.find_first_undo_candidate_index(&toggled_groups);
        // the `false` below: don't invert undos since our first_candidate is based on the current undo set, not past
        let mut deletes_from_union =
            self.deletes_from_union_before_index(first_candidate, false).into_owned();

        for rev in &self.revs[first_candidate..] {
            if let Edit { ref undo_group, ref inserts, ref deletes, .. } = rev.edit {
                if groups.contains(undo_group) {
                    if !inserts.is_empty() {
                        deletes_from_union = deletes_from_union.transform_union(inserts);
                    }
                } else {
                    if !inserts.is_empty() {
                        deletes_from_union = deletes_from_union.transform_expand(inserts);
                    }
                    if !deletes.is_empty() {
                        deletes_from_union = deletes_from_union.union(deletes);
                    }
                }
            }
        }

        let deletes_bitxor = self.deletes_from_union.bitxor(&deletes_from_union);
        let max_undo_so_far = self.revs.last().unwrap().max_undo_so_far;
        (
            Revision {
                rev_id: self.next_rev_id(),
                max_undo_so_far,
                edit: Undo { toggled_groups, deletes_bitxor },
            },
            deletes_from_union,
        )
    }

    // TODO: maybe refactor this API to take a toggle set
    pub fn undo(&mut self, groups: BTreeSet<usize>) {
        let (new_rev, new_deletes_from_union) = self.compute_undo(&groups);

        let (new_text, new_tombstones) = shuffle(
            &self.text,
            &self.tombstones,
            &self.deletes_from_union,
            &new_deletes_from_union,
        );

        self.text = new_text;
        self.tombstones = new_tombstones;
        self.deletes_from_union = new_deletes_from_union;
        self.undone_groups = groups;
        self.revs.push(new_rev);
        self.rev_id_counter += 1;
    }

    pub fn is_equivalent_revision(&self, base_rev: RevId, other_rev: RevId) -> bool {
        let base_subset = self
            .find_rev(base_rev)
            .map(|rev_index| self.deletes_from_cur_union_for_index(rev_index));
        let other_subset = self
            .find_rev(other_rev)
            .map(|rev_index| self.deletes_from_cur_union_for_index(rev_index));

        base_subset.is_some() && base_subset == other_subset
    }

    // Note: this function would need some work to handle retaining arbitrary revisions,
    // partly because the reachability calculation would become more complicated (a
    // revision might hold content from an undo group that would otherwise be gc'ed),
    // and partly because you need to retain more undo history, to supply input to the
    // reachability calculation.
    //
    // Thus, it's easiest to defer gc to when all plugins quiesce, but it's certainly
    // possible to fix it so that's not necessary.
    pub fn gc(&mut self, gc_groups: &BTreeSet<usize>) {
        let mut gc_dels = self.empty_subset_before_first_rev();
        // TODO: want to let caller retain more rev_id's.
        let mut retain_revs = BTreeSet::new();
        if let Some(last) = self.revs.last() {
            retain_revs.insert(last.rev_id);
        }
        {
            for rev in &self.revs {
                if let Edit { ref undo_group, ref inserts, ref deletes, .. } = rev.edit {
                    if !retain_revs.contains(&rev.rev_id) && gc_groups.contains(undo_group) {
                        if self.undone_groups.contains(undo_group) {
                            if !inserts.is_empty() {
                                gc_dels = gc_dels.transform_union(inserts);
                            }
                        } else {
                            if !inserts.is_empty() {
                                gc_dels = gc_dels.transform_expand(inserts);
                            }
                            if !deletes.is_empty() {
                                gc_dels = gc_dels.union(deletes);
                            }
                        }
                    } else if !inserts.is_empty() {
                        gc_dels = gc_dels.transform_expand(inserts);
                    }
                }
            }
        }
        if !gc_dels.is_empty() {
            let not_in_tombstones = self.deletes_from_union.complement();
            let dels_from_tombstones = gc_dels.transform_shrink(&not_in_tombstones);
            self.tombstones = dels_from_tombstones.delete_from(&self.tombstones);
            self.deletes_from_union = self.deletes_from_union.transform_shrink(&gc_dels);
        }
        let old_revs = std::mem::replace(&mut self.revs, Vec::new());
        for rev in old_revs.into_iter().rev() {
            match rev.edit {
                Edit { priority, undo_group, inserts, deletes } => {
                    let new_gc_dels = if inserts.is_empty() {
                        None
                    } else {
                        Some(gc_dels.transform_shrink(&inserts))
                    };
                    if retain_revs.contains(&rev.rev_id) || !gc_groups.contains(&undo_group) {
                        let (inserts, deletes) = if gc_dels.is_empty() {
                            (inserts, deletes)
                        } else {
                            (inserts.transform_shrink(&gc_dels), deletes.transform_shrink(&gc_dels))
                        };
                        self.revs.push(Revision {
                            rev_id: rev.rev_id,
                            max_undo_so_far: rev.max_undo_so_far,
                            edit: Edit { priority, undo_group, inserts, deletes },
                        });
                    }
                    if let Some(new_gc_dels) = new_gc_dels {
                        gc_dels = new_gc_dels;
                    }
                }
                Undo { toggled_groups, deletes_bitxor } => {
                    // We're super-aggressive about dropping these; after gc, the history
                    // of which undos were used to compute deletes_from_union in edits may be lost.
                    if retain_revs.contains(&rev.rev_id) {
                        let new_deletes_bitxor = if gc_dels.is_empty() {
                            deletes_bitxor
                        } else {
                            deletes_bitxor.transform_shrink(&gc_dels)
                        };
                        self.revs.push(Revision {
                            rev_id: rev.rev_id,
                            max_undo_so_far: rev.max_undo_so_far,
                            edit: Undo {
                                toggled_groups: &toggled_groups - gc_groups,
                                deletes_bitxor: new_deletes_bitxor,
                            },
                        })
                    }
                }
            }
        }
        self.revs.reverse();
    }

    /// Merge the new content from another Engine into this one with a CRDT merge
    pub fn merge(&mut self, other: &Engine) {
        let (mut new_revs, text, tombstones, deletes_from_union) = {
            let base_index = find_base_index(&self.revs, &other.revs);
            let a_to_merge = &self.revs[base_index..];
            let b_to_merge = &other.revs[base_index..];

            let common = find_common(a_to_merge, b_to_merge);

            let a_new = rearrange(a_to_merge, &common, self.deletes_from_union.len());
            let b_new = rearrange(b_to_merge, &common, other.deletes_from_union.len());

            let b_deltas =
                compute_deltas(&b_new, &other.text, &other.tombstones, &other.deletes_from_union);
            let expand_by = compute_transforms(a_new);

            let max_undo = self.max_undo_group_id();
            rebase(
                expand_by,
                b_deltas,
                self.text.clone(),
                self.tombstones.clone(),
                self.deletes_from_union.clone(),
                max_undo,
            )
        };

        self.text = text;
        self.tombstones = tombstones;
        self.deletes_from_union = deletes_from_union;
        self.revs.append(&mut new_revs);
    }

    /// When merging between multiple concurrently-editing sessions, each session should have a unique ID
    /// set with this function, which will make the revisions they create not have colliding IDs.
    /// For safety, this will panic if any revisions have already been added to the Engine.
    ///
    /// Merge may panic or return incorrect results if session IDs collide, which is why they can be
    /// 96 bits which is more than sufficient for this to never happen.
    pub fn set_session_id(&mut self, session: SessionId) {
        assert_eq!(
            1,
            self.revs.len(),
            "Revisions were added to an Engine before set_session_id, these may collide."
        );
        self.session = session;
    }
}

// ======== Generic helpers

/// Move sections from text to tombstones and out of tombstones based on a new and old set of deletions
fn shuffle_tombstones(
    text: &Rope,
    tombstones: &Rope,
    old_deletes_from_union: &Subset,
    new_deletes_from_union: &Subset,
) -> Rope {
    // Taking the complement of deletes_from_union leads to an interleaving valid for swapped text and tombstones,
    // allowing us to use the same method to insert the text into the tombstones.
    let inverse_tombstones_map = old_deletes_from_union.complement();
    let move_delta =
        Delta::synthesize(text, &inverse_tombstones_map, &new_deletes_from_union.complement());
    move_delta.apply(tombstones)
}

/// Move sections from text to tombstones and vice versa based on a new and old set of deletions.
/// Returns a tuple of a new text `Rope` and a new `Tombstones` rope described by `new_deletes_from_union`.
fn shuffle(
    text: &Rope,
    tombstones: &Rope,
    old_deletes_from_union: &Subset,
    new_deletes_from_union: &Subset,
) -> (Rope, Rope) {
    // Delta that deletes the right bits from the text
    let del_delta = Delta::synthesize(tombstones, old_deletes_from_union, new_deletes_from_union);
    let new_text = del_delta.apply(text);
    // println!("shuffle: old={:?} new={:?} old_text={:?} new_text={:?} old_tombstones={:?}",
    //     old_deletes_from_union, new_deletes_from_union, text, new_text, tombstones);
    (new_text, shuffle_tombstones(text, tombstones, old_deletes_from_union, new_deletes_from_union))
}

// ======== Merge helpers

/// Find an index before which everything is the same
fn find_base_index(a: &[Revision], b: &[Revision]) -> usize {
    assert!(!a.is_empty() && !b.is_empty());
    assert!(a[0].rev_id == b[0].rev_id);
    // TODO find the maximum base revision.
    // this should have the same behavior, but worse performance
    1
}

/// Find a set of revisions common to both lists
fn find_common(a: &[Revision], b: &[Revision]) -> BTreeSet<RevId> {
    // TODO make this faster somehow?
    let a_ids: BTreeSet<RevId> = a.iter().map(|r| r.rev_id).collect();
    let b_ids: BTreeSet<RevId> = b.iter().map(|r| r.rev_id).collect();
    a_ids.intersection(&b_ids).cloned().collect()
}

/// Returns the operations in `revs` that don't have their `rev_id` in
/// `base_revs`, but modified so that they are in the same order but based on
/// the `base_revs`. This allows the rest of the merge to operate on only
/// revisions not shared by both sides.
///
/// Conceptually, see the diagram below, with `.` being base revs and `n` being
/// non-base revs, `N` being transformed non-base revs, and rearranges it:
/// .n..n...nn..  -> ........NNNN -> returns vec![N,N,N,N]
fn rearrange(revs: &[Revision], base_revs: &BTreeSet<RevId>, head_len: usize) -> Vec<Revision> {
    // transform representing the characters added by common revisions after a point.
    let mut s = Subset::new(head_len);

    let mut out = Vec::with_capacity(revs.len() - base_revs.len());
    for rev in revs.iter().rev() {
        let is_base = base_revs.contains(&rev.rev_id);
        let contents = match rev.edit {
            Contents::Edit { priority, undo_group, ref inserts, ref deletes } => {
                if is_base {
                    s = inserts.transform_union(&s);
                    None
                } else {
                    // fast-forward this revision over all common ones after it
                    let transformed_inserts = inserts.transform_expand(&s);
                    let transformed_deletes = deletes.transform_expand(&s);
                    // we don't want new revisions before this to be transformed after us
                    s = s.transform_shrink(&transformed_inserts);
                    Some(Contents::Edit {
                        inserts: transformed_inserts,
                        deletes: transformed_deletes,
                        priority,
                        undo_group,
                    })
                }
            }
            Contents::Undo { .. } => panic!("can't merge undo yet"),
        };
        if let Some(edit) = contents {
            out.push(Revision { edit, rev_id: rev.rev_id, max_undo_so_far: rev.max_undo_so_far });
        }
    }

    out.as_mut_slice().reverse();
    out
}

#[derive(Clone, Debug)]
struct DeltaOp {
    rev_id: RevId,
    priority: usize,
    undo_group: usize,
    inserts: InsertDelta<RopeInfo>,
    deletes: Subset,
}

/// Transform `revs`, which doesn't include information on the actual content of the operations,
/// into an `InsertDelta`-based representation that does by working backward from the text and tombstones.
fn compute_deltas(
    revs: &[Revision],
    text: &Rope,
    tombstones: &Rope,
    deletes_from_union: &Subset,
) -> Vec<DeltaOp> {
    let mut out = Vec::with_capacity(revs.len());

    let mut cur_all_inserts = Subset::new(deletes_from_union.len());
    for rev in revs.iter().rev() {
        match rev.edit {
            Contents::Edit { priority, undo_group, ref inserts, ref deletes } => {
                let older_all_inserts = inserts.transform_union(&cur_all_inserts);

                // TODO could probably be more efficient by avoiding shuffling from head every time
                let tombstones_here =
                    shuffle_tombstones(text, tombstones, deletes_from_union, &older_all_inserts);
                let delta =
                    Delta::synthesize(&tombstones_here, &older_all_inserts, &cur_all_inserts);
                // TODO create InsertDelta directly and more efficiently instead of factoring
                let (ins, _) = delta.factor();
                out.push(DeltaOp {
                    rev_id: rev.rev_id,
                    priority,
                    undo_group,
                    inserts: ins,
                    deletes: deletes.clone(),
                });

                cur_all_inserts = older_all_inserts;
            }
            Contents::Undo { .. } => panic!("can't merge undo yet"),
        }
    }

    out.as_mut_slice().reverse();
    out
}

/// Computes a series of priorities and transforms for the deltas on the right
/// from the new revisions on the left.
///
/// Applies an optimization where it combines sequential revisions with the
/// same priority into one transform to decrease the number of transforms that
/// have to be considered in `rebase` substantially for normal editing
/// patterns. Any large runs of typing in the same place by the same user (e.g
/// typing a paragraph) will be combined into a single segment in a transform
/// as opposed to thousands of revisions.
fn compute_transforms(revs: Vec<Revision>) -> Vec<(FullPriority, Subset)> {
    let mut out = Vec::new();
    let mut last_priority: Option<usize> = None;
    for r in revs {
        if let Contents::Edit { priority, inserts, .. } = r.edit {
            if inserts.is_empty() {
                continue;
            }
            if Some(priority) == last_priority {
                let last: &mut (FullPriority, Subset) = out.last_mut().unwrap();
                last.1 = last.1.transform_union(&inserts);
            } else {
                last_priority = Some(priority);
                let prio = FullPriority { priority, session_id: r.rev_id.session_id() };
                out.push((prio, inserts));
            }
        }
    }
    out
}

/// Rebase `b_new` on top of `expand_by` and return revision contents that can be appended as new
/// revisions on top of the revisions represented by `expand_by`.
fn rebase(
    mut expand_by: Vec<(FullPriority, Subset)>,
    b_new: Vec<DeltaOp>,
    mut text: Rope,
    mut tombstones: Rope,
    mut deletes_from_union: Subset,
    mut max_undo_so_far: usize,
) -> (Vec<Revision>, Rope, Rope, Subset) {
    let mut out = Vec::with_capacity(b_new.len());

    let mut next_expand_by = Vec::with_capacity(expand_by.len());
    for op in b_new {
        let DeltaOp { rev_id, priority, undo_group, mut inserts, mut deletes } = op;
        let full_priority = FullPriority { priority, session_id: rev_id.session_id() };
        // expand by each in expand_by
        for &(trans_priority, ref trans_inserts) in &expand_by {
            let after = full_priority >= trans_priority; // should never be ==
                                                         // d-expand by other
            inserts = inserts.transform_expand(trans_inserts, after);
            // trans-expand other by expanded so they have the same context
            let inserted = inserts.inserted_subset();
            let new_trans_inserts = trans_inserts.transform_expand(&inserted);
            // The deletes are already after our inserts, but we need to include the other inserts
            deletes = deletes.transform_expand(&new_trans_inserts);
            // On the next step we want things in expand_by to have op in the context
            next_expand_by.push((trans_priority, new_trans_inserts));
        }

        let text_inserts = inserts.transform_shrink(&deletes_from_union);
        let text_with_inserts = text_inserts.apply(&text);
        let inserted = inserts.inserted_subset();

        let expanded_deletes_from_union = deletes_from_union.transform_expand(&inserted);
        let new_deletes_from_union = expanded_deletes_from_union.union(&deletes);
        let (new_text, new_tombstones) = shuffle(
            &text_with_inserts,
            &tombstones,
            &expanded_deletes_from_union,
            &new_deletes_from_union,
        );

        text = new_text;
        tombstones = new_tombstones;
        deletes_from_union = new_deletes_from_union;

        max_undo_so_far = std::cmp::max(max_undo_so_far, undo_group);
        out.push(Revision {
            rev_id,
            max_undo_so_far,
            edit: Contents::Edit { priority, undo_group, deletes, inserts: inserted },
        });

        expand_by = next_expand_by;
        next_expand_by = Vec::with_capacity(expand_by.len());
    }

    (out, text, tombstones, deletes_from_union)
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::MissingRevision(_) => write!(f, "Revision not found"),
            Error::MalformedDelta { delta_len, rev_len } => {
                write!(f, "Delta base_len {} does not match revision length {}", delta_len, rev_len)
            }
        }
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
#[rustfmt::skip]
mod tests {
    use crate::engine::*;
    use crate::rope::{Rope, RopeInfo};
    use crate::delta::{Builder, Delta, DeltaElement};
    use crate::multiset::Subset;
    use crate::interval::Interval;
    use std::collections::BTreeSet;
    use crate::test_helpers::{parse_subset_list, parse_subset, parse_delta, debug_subsets};

    const TEST_STR: &'static str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

    fn build_delta_1() -> Delta<RopeInfo> {
        let mut d_builder = Builder::new(TEST_STR.len());
        d_builder.delete(Interval::new(10, 36));
        d_builder.replace(Interval::new(39, 42), Rope::from("DEEF"));
        d_builder.replace(Interval::new(54, 54), Rope::from("999"));
        d_builder.delete(Interval::new(58, 61));
        d_builder.build()
    }

    fn build_delta_2() -> Delta<RopeInfo> {
        let mut d_builder = Builder::new(TEST_STR.len());
        d_builder.replace(Interval::new(1, 3), Rope::from("!"));
        d_builder.delete(Interval::new(10, 36));
        d_builder.replace(Interval::new(42, 45), Rope::from("GI"));
        d_builder.replace(Interval::new(54, 54), Rope::from("888"));
        d_builder.replace(Interval::new(59, 60), Rope::from("HI"));
        d_builder.build()
    }

    #[test]
    fn edit_rev_simple() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        let first_rev = engine.get_head_rev_id().token();
        engine.edit_rev(0, 1, first_rev, build_delta_1());
        assert_eq!("0123456789abcDEEFghijklmnopqr999stuvz", String::from(engine.get_head()));
    }

    #[test]
    fn edit_rev_empty() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        let first_rev = engine.get_head_rev_id().token();
        let delta = Delta {
            base_len: TEST_STR.len(),
            els: vec![DeltaElement::Copy(0, TEST_STR.len())],
        };
        engine.edit_rev(0, 1, first_rev, delta.clone());
        assert_eq!(TEST_STR, String::from(engine.get_head()));
        engine.edit_rev(0, 1, first_rev, delta.clone());
        assert_eq!(TEST_STR, String::from(engine.get_head()));
    }

    #[test]
    fn edit_rev_concurrent() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        let first_rev = engine.get_head_rev_id().token();
        engine.edit_rev(1, 1, first_rev, build_delta_1());
        engine.edit_rev(0, 2, first_rev, build_delta_2());
        assert_eq!("0!3456789abcDEEFGIjklmnopqr888999stuvHIz", String::from(engine.get_head()));
    }

    #[test]
    #[should_panic(expected = "Delta base_len 5 does not match revision length 6")]
    fn edit_rev_bad_delta_len() {
        let test_str = "hello";
        let mut engine = Engine::new(Rope::from(test_str));
        let iv = Interval::new(1, 1);

        let mut builder = Builder::new(test_str.len());
        builder.replace(iv, "1".into());
        let delta1 = builder.build();

        let mut builder = Builder::new(test_str.len());
        builder.replace(iv, "2".into());
        let delta2 = builder.build();

        let rev = engine.get_head_rev_id().token();
        engine.edit_rev(1, 1, rev, delta1);

        // this second delta now has an incorrect length for the engine
        let rev = engine.get_head_rev_id().token();
        engine.edit_rev(1, 2, rev, delta2);
    }

    fn undo_test(before: bool, undos : BTreeSet<usize>, output: &str) {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        let first_rev = engine.get_head_rev_id().token();
        if before {
            engine.undo(undos.clone());
        }
        engine.edit_rev(1, 1, first_rev, build_delta_1());
        engine.edit_rev(0, 2, first_rev, build_delta_2());
        if !before {
            engine.undo(undos);
        }
        assert_eq!(output, String::from(engine.get_head()));
    }

    #[test]
    fn edit_rev_undo() {
        undo_test(true, [1,2].iter().cloned().collect(), TEST_STR);
    }

    #[test]
    fn edit_rev_undo_2() {
        undo_test(true, [2].iter().cloned().collect(), "0123456789abcDEEFghijklmnopqr999stuvz");
    }

    #[test]
    fn edit_rev_undo_3() {
        undo_test(true, [1].iter().cloned().collect(), "0!3456789abcdefGIjklmnopqr888stuvwHIyz");
    }

    #[test]
    fn try_delta_rev_head() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        let first_rev = engine.get_head_rev_id().token();
        engine.edit_rev(1, 1, first_rev, build_delta_1());
        let d = engine.try_delta_rev_head(first_rev).unwrap();
        assert_eq!(String::from(engine.get_head()), d.apply_to_string(TEST_STR));
    }

    #[test]
    fn try_delta_rev_head_2() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        let first_rev = engine.get_head_rev_id().token();
        engine.edit_rev(1, 1, first_rev, build_delta_1());
        engine.edit_rev(0, 2, first_rev, build_delta_2());
        let d = engine.try_delta_rev_head(first_rev).unwrap();
        assert_eq!(String::from(engine.get_head()), d.apply_to_string(TEST_STR));
    }

    #[test]
    fn try_delta_rev_head_3() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        let first_rev = engine.get_head_rev_id().token();
        engine.edit_rev(1, 1, first_rev, build_delta_1());
        let after_first_edit = engine.get_head_rev_id().token();
        engine.edit_rev(0, 2, first_rev, build_delta_2());
        let d = engine.try_delta_rev_head(after_first_edit).unwrap();
        assert_eq!(String::from(engine.get_head()), d.apply_to_string("0123456789abcDEEFghijklmnopqr999stuvz"));
    }

    #[test]
    fn try_delta_rev_head_missing_token() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        let first_rev = engine.get_head_rev_id().token();
        let bad_rev = RevToken::default();
        engine.edit_rev(1, 1, first_rev, build_delta_1());
        let d = engine.try_delta_rev_head(bad_rev);
        assert!(d.is_err());
    }

    #[test]
    fn undo() {
        undo_test(false, [1,2].iter().cloned().collect(), TEST_STR);
    }

    #[test]
    fn undo_2() {
        undo_test(false, [2].iter().cloned().collect(), "0123456789abcDEEFghijklmnopqr999stuvz");
    }

    #[test]
    fn undo_3() {
        undo_test(false, [1].iter().cloned().collect(), "0!3456789abcdefGIjklmnopqr888stuvwHIyz");
    }

    #[test]
    fn undo_4() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        let d1 = Delta::simple_edit(Interval::new(0,0), Rope::from("a"), TEST_STR.len());
        let first_rev = engine.get_head_rev_id().token();
        engine.edit_rev(1, 1, first_rev, d1.clone());
        let new_head = engine.get_head_rev_id().token();
        engine.undo([1].iter().cloned().collect());
        let d2 = Delta::simple_edit(Interval::new(0,0), Rope::from("a"), TEST_STR.len()+1);
        engine.edit_rev(1, 2, new_head, d2); // note this is based on d1 before, not the undo
        let new_head_2 = engine.get_head_rev_id().token();
        let d3 = Delta::simple_edit(Interval::new(0,0), Rope::from("b"), TEST_STR.len()+1);
        engine.edit_rev(1, 3, new_head_2, d3);
        engine.undo([1,3].iter().cloned().collect());
        assert_eq!("a0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz", String::from(engine.get_head()));
    }

    #[test]
    fn undo_5() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        let d1 = Delta::simple_edit(Interval::new(0,10), Rope::from(""), TEST_STR.len());
        let first_rev = engine.get_head_rev_id().token();
        engine.edit_rev(1, 1, first_rev, d1.clone());
        engine.edit_rev(1, 2, first_rev, d1.clone());
        engine.undo([1].iter().cloned().collect());
        assert_eq!("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz", String::from(engine.get_head()));
        engine.undo([1,2].iter().cloned().collect());
        assert_eq!(TEST_STR, String::from(engine.get_head()));
        engine.undo([].iter().cloned().collect());
        assert_eq!("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz", String::from(engine.get_head()));
    }

    #[test]
    fn gc() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        let d1 = Delta::simple_edit(Interval::new(0,0), Rope::from("c"), TEST_STR.len());
        let first_rev = engine.get_head_rev_id().token();
        engine.edit_rev(1, 1, first_rev, d1);
        let new_head = engine.get_head_rev_id().token();
        engine.undo([1].iter().cloned().collect());
        let d2 = Delta::simple_edit(Interval::new(0,0), Rope::from("a"), TEST_STR.len()+1);
        engine.edit_rev(1, 2, new_head, d2);
        let gc : BTreeSet<usize> = [1].iter().cloned().collect();
        engine.gc(&gc);
        let d3 = Delta::simple_edit(Interval::new(0,0), Rope::from("b"), TEST_STR.len()+1);
        let new_head_2 = engine.get_head_rev_id().token();
        engine.edit_rev(1, 3, new_head_2, d3);
        engine.undo([3].iter().cloned().collect());
        assert_eq!("a0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz", String::from(engine.get_head()));
    }

    /// This case is a regression test reproducing a panic I found while using the UI.
    /// It does undos and gcs in a pattern that can actually happen when using the editor.
    fn gc_scenario(edits: usize, max_undos: usize) {
        let mut engine = Engine::new(Rope::from(""));

        // insert `edits` letter "b"s in separate undo groups
        for i in 0..edits {
            let d = Delta::simple_edit(Interval::new(0,0), Rope::from("b"), i);
            let head = engine.get_head_rev_id().token();
            engine.edit_rev(1, i+1, head, d);
            if i >= max_undos {
                let to_gc : BTreeSet<usize> = [i-max_undos].iter().cloned().collect();
                engine.gc(&to_gc)
            }
        }

        // spam cmd+z until the available undo history is exhausted
        let mut to_undo = BTreeSet::new();
        for i in ((edits-max_undos)..edits).rev() {
            to_undo.insert(i+1);
            engine.undo(to_undo.clone());
        }

        // insert a character at the beginning
        let d1 = Delta::simple_edit(Interval::new(0,0), Rope::from("h"), engine.get_head().len());
        let head = engine.get_head_rev_id().token();
        engine.edit_rev(1, edits+1, head, d1);

        // since character was inserted after gc, editor gcs all undone things
        engine.gc(&to_undo);

        // insert character at end, when this test was added, it panic'd here
        let chars_left = (edits-max_undos)+1;
        let d2 = Delta::simple_edit(Interval::new(chars_left, chars_left), Rope::from("f"), engine.get_head().len());
        let head2 = engine.get_head_rev_id().token();
        engine.edit_rev(1, edits+1, head2, d2);

        let mut soln = String::from("h");
        for _ in 0..(edits-max_undos) {
            soln.push('b');
        }
        soln.push('f');
        assert_eq!(soln, String::from(engine.get_head()));
    }

    #[test]
    fn gc_2() {
        // the smallest values with which it still fails:
        gc_scenario(4,3);
    }

    #[test]
    fn gc_3() {
        // original values this test was created/found with in the UI:
        gc_scenario(35,20);
    }

    #[test]
    fn gc_4() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        let d1 = Delta::simple_edit(Interval::new(0,10), Rope::from(""), TEST_STR.len());
        let first_rev = engine.get_head_rev_id().token();
        engine.edit_rev(1, 1, first_rev, d1.clone());
        engine.edit_rev(1, 2, first_rev, d1.clone());
        let gc : BTreeSet<usize> = [1].iter().cloned().collect();
        engine.gc(&gc);
        // shouldn't do anything since it was double-deleted and one was GC'd
        engine.undo([1,2].iter().cloned().collect());
        assert_eq!("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz", String::from(engine.get_head()));
    }

    #[test]
    fn gc_5() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        let d1 = Delta::simple_edit(Interval::new(0,10), Rope::from(""), TEST_STR.len());
        let initial_rev = engine.get_head_rev_id().token();
        engine.undo([1].iter().cloned().collect());
        engine.edit_rev(1, 1, initial_rev, d1.clone());
        engine.edit_rev(1, 2, initial_rev, d1.clone());
        let gc : BTreeSet<usize> = [1].iter().cloned().collect();
        engine.gc(&gc);
        // only one of the deletes was gc'd, the other should still be in effect
        assert_eq!("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz", String::from(engine.get_head()));
        // since one of the two deletes was gc'd this should undo the one that wasn't
        engine.undo([2].iter().cloned().collect());
        assert_eq!(TEST_STR, String::from(engine.get_head()));
    }

    #[test]
    fn gc_6() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        let d1 = Delta::simple_edit(Interval::new(0,10), Rope::from(""), TEST_STR.len());
        let initial_rev = engine.get_head_rev_id().token();
        engine.edit_rev(1, 1, initial_rev, d1.clone());
        engine.undo([1,2].iter().cloned().collect());
        engine.edit_rev(1, 2, initial_rev, d1.clone());
        let gc : BTreeSet<usize> = [1].iter().cloned().collect();
        engine.gc(&gc);
        assert_eq!(TEST_STR, String::from(engine.get_head()));
        // since one of the two deletes was gc'd this should re-do the one that wasn't
        engine.undo([].iter().cloned().collect());
        assert_eq!("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz", String::from(engine.get_head()));
    }

    fn basic_rev(i: usize) -> RevId {
        RevId { session1: 1, session2: 0, num: i as u32 }
    }

    fn basic_insert_ops(inserts: Vec<Subset>, priority: usize) -> Vec<Revision> {
        inserts.into_iter().enumerate().map(|(i, inserts)| {
            let deletes = Subset::new(inserts.len());
            Revision {
                rev_id: basic_rev(i+1),
                max_undo_so_far: i+1,
                edit: Contents::Edit {
                    priority, inserts, deletes,
                    undo_group: i+1,
                }
            }
        }).collect()
    }

    #[test]
    fn rearrange_1() {
        let inserts = parse_subset_list("
        ##
        -#-
        #---
        ---#-
        -----#
        #------
        ");
        let revs = basic_insert_ops(inserts, 1);
        let base: BTreeSet<RevId> = [3,5].iter().cloned().map(basic_rev).collect();

        let rearranged = rearrange(&revs, &base, 7);
        let rearranged_inserts: Vec<Subset> = rearranged.into_iter().map(|c| {
            match c.edit {
                Contents::Edit {inserts, ..} => inserts,
                Contents::Undo { .. } => panic!(),
            }
        }).collect();

        debug_subsets(&rearranged_inserts);
        let correct = parse_subset_list("
        -##-
        --#--
        ---#--
        #------
        ");
        assert_eq!(correct, rearranged_inserts);
    }

    fn ids_to_fake_revs(ids: &[usize]) -> Vec<Revision> {
        let contents = Contents::Edit {
            priority: 0,
            undo_group: 0,
            inserts: Subset::new(0),
            deletes: Subset::new(0),
        };

        ids.iter().cloned().map(|i| {
            Revision {
                rev_id: basic_rev(i),
                max_undo_so_far: i,
                edit: contents.clone()
            }
        }).collect()
    }

    #[test]
    fn find_common_1() {
        let a: Vec<Revision> = ids_to_fake_revs(&[0,2,4,6,8,10,12]);
        let b: Vec<Revision> = ids_to_fake_revs(&[0,1,2,4,5,8,9]);
        let res = find_common(&a, &b);

        let correct: BTreeSet<RevId> = [0,2,4,8].iter().cloned().map(basic_rev).collect();
        assert_eq!(correct, res);
    }


    #[test]
    fn find_base_1() {
        let a: Vec<Revision> = ids_to_fake_revs(&[0,2,4,6,8,10,12]);
        let b: Vec<Revision> = ids_to_fake_revs(&[0,1,2,4,5,8,9]);
        let res = find_base_index(&a, &b);

        assert_eq!(1, res);
    }

    #[test]
    fn compute_deltas_1() {
        let inserts = parse_subset_list("
        -##-
        --#--
        ---#--
        #------
        ");
        let revs = basic_insert_ops(inserts, 1);

        let text = Rope::from("13456");
        let tombstones = Rope::from("27");
        let deletes_from_union = parse_subset("-#----#");
        let delta_ops = compute_deltas(&revs, &text, &tombstones, &deletes_from_union);

        println!("{:#?}", delta_ops);

        let mut r = Rope::from("27");
        for op in &delta_ops {
            r = op.inserts.apply(&r);
        }
        assert_eq!("1234567", String::from(r));
    }

    #[test]
    fn compute_transforms_1() {
        let inserts = parse_subset_list("
        -##-
        --#--
        ---#--
        #------
        ");
        let revs = basic_insert_ops(inserts, 1);

        let expand_by = compute_transforms(revs);
        assert_eq!(1, expand_by.len());
        assert_eq!(1, expand_by[0].0.priority);
        let subset_str = format!("{:#?}", expand_by[0].1);
        assert_eq!("#-####-", &subset_str);
    }

    #[test]
    fn compute_transforms_2() {
        let inserts_1 = parse_subset_list("
        -##-
        --#--
        ");
        let mut revs = basic_insert_ops(inserts_1, 1);
        let inserts_2 = parse_subset_list("
        ----
        ");
        let mut revs_2 = basic_insert_ops(inserts_2, 4);
        revs.append(&mut revs_2);
        let inserts_3 = parse_subset_list("
        ---#--
        #------
        ");
        let mut revs_3 = basic_insert_ops(inserts_3, 2);
        revs.append(&mut revs_3);

        let expand_by = compute_transforms(revs);
        assert_eq!(2, expand_by.len());
        assert_eq!(1, expand_by[0].0.priority);
        assert_eq!(2, expand_by[1].0.priority);

        let subset_str = format!("{:#?}", expand_by[0].1);
        assert_eq!("-###-", &subset_str);
        let subset_str = format!("{:#?}", expand_by[1].1);
        assert_eq!("#---#--", &subset_str);
    }

    #[test]
    fn rebase_1() {
        let inserts = parse_subset_list("
        --#-
        ----#
        ");
        let a_revs = basic_insert_ops(inserts.clone(), 1);
        let b_revs = basic_insert_ops(inserts, 2);

        let text_b = Rope::from("zpbj");
        let tombstones_b = Rope::from("a");
        let deletes_from_union_b = parse_subset("-#---");
        let b_delta_ops = compute_deltas(&b_revs, &text_b, &tombstones_b, &deletes_from_union_b);

        println!("{:#?}", b_delta_ops);

        let text_a = Rope::from("zcbd");
        let tombstones_a = Rope::from("a");
        let deletes_from_union_a = parse_subset("-#---");
        let expand_by = compute_transforms(a_revs);

        let (revs, text_2, tombstones_2, deletes_from_union_2) =
            rebase(expand_by, b_delta_ops, text_a, tombstones_a, deletes_from_union_a, 0);

        let rebased_inserts: Vec<Subset> = revs.into_iter().map(|c| {
            match c.edit {
                Contents::Edit {inserts, ..} => inserts,
                Contents::Undo { .. } => panic!(),
            }
        }).collect();

        debug_subsets(&rebased_inserts);
        let correct = parse_subset_list("
        ---#--
        ------#
        ");
        assert_eq!(correct, rebased_inserts);


        assert_eq!("zcpbdj", String::from(&text_2));
        assert_eq!("a", String::from(&tombstones_2));
        assert_eq!("-#-----", format!("{:#?}", deletes_from_union_2));
    }

    // ============== Merge script tests

    #[derive(Clone, Debug)]
    enum MergeTestOp {
        Merge(usize, usize),
        Assert(usize, String),
        AssertAll(String),
        AssertMaxUndoSoFar(usize, usize),
        Edit { ei: usize, p: usize, u: usize, d: Delta<RopeInfo> },
    }

    #[derive(Debug)]
    struct MergeTestState {
        peers: Vec<Engine>,
    }

    impl MergeTestState {
        fn new(count: usize) -> MergeTestState {
            let mut peers = Vec::with_capacity(count);
            for i in 0..count {
                let mut peer = Engine::new(Rope::from(""));
                peer.set_session_id(((i*1000) as u64, 0));
                peers.push(peer);
            }
            MergeTestState { peers }
        }

        fn run_op(&mut self, op: &MergeTestOp) {
            match *op {
                MergeTestOp::Merge(ai, bi) => {
                    let (start, end) = self.peers.split_at_mut(ai);
                    let (a, rest) = end.split_first_mut().unwrap();
                    let b = if bi < ai {
                        &mut start[bi]
                    } else {
                        &mut rest[bi - ai - 1]
                    };
                    a.merge(b);
                },
                MergeTestOp::Assert(ei, ref correct) => {
                    let e = &mut self.peers[ei];
                    assert_eq!(correct, &String::from(e.get_head()), "for peer {}", ei);
                },
                MergeTestOp::AssertMaxUndoSoFar(ei, correct) => {
                    let e = &mut self.peers[ei];
                    assert_eq!(correct, e.max_undo_group_id(), "for peer {}", ei);
                },
                MergeTestOp::AssertAll(ref correct) => {
                    for (ei, e) in self.peers.iter().enumerate() {
                        assert_eq!(correct, &String::from(e.get_head()), "for peer {}", ei);
                    }
                },
                MergeTestOp::Edit { ei, p, u, d: ref delta } => {
                    let e = &mut self.peers[ei];
                    let head = e.get_head_rev_id().token();
                    e.edit_rev(p, u, head, delta.clone());
                },
            }
        }

        fn run_script(&mut self, script: &[MergeTestOp]) {
            for (i, op) in script.iter().enumerate() {
                println!("running {:?} at index {}", op, i);
                self.run_op(op);
            }
        }
    }

    /// Like the scanned whiteboard diagram I have, but without deleting 'a'
    #[test]
    fn merge_insert_only_whiteboard() {
        use self::MergeTestOp::*;
        let script = vec![
            Edit { ei: 2, p: 1, u: 1, d: parse_delta("ab") },
            Merge(0,2), Merge(1, 2),
            Assert(0, "ab".to_owned()),
            Assert(1, "ab".to_owned()),
            Assert(2, "ab".to_owned()),
            Edit { ei: 0, p: 3, u: 1, d: parse_delta("-c-") },
            Edit { ei: 0, p: 3, u: 1, d: parse_delta("---d") },
            Assert(0, "acbd".to_owned()),
            Edit { ei: 1, p: 5, u: 1, d: parse_delta("-p-") },
            Edit { ei: 1, p: 5, u: 1, d: parse_delta("---j") },
            Assert(1, "apbj".to_owned()),
            Edit { ei: 2, p: 1, u: 1, d: parse_delta("z--") },
            Merge(0,2), Merge(1, 2),
            Assert(0, "zacbd".to_owned()),
            Assert(1, "zapbj".to_owned()),
            Merge(0,1),
            Assert(0, "zacpbdj".to_owned()),
        ];
        MergeTestState::new(3).run_script(&script[..]);
    }

    /// Tests that priorities are used to break ties correctly
    #[test]
    fn merge_priorities() {
        use self::MergeTestOp::*;
        let script = vec![
            Edit { ei: 2, p: 1, u: 1, d: parse_delta("ab") },
            Merge(0,2), Merge(1, 2),
            Assert(0, "ab".to_owned()),
            Assert(1, "ab".to_owned()),
            Assert(2, "ab".to_owned()),
            Edit { ei: 0, p: 3, u: 1, d: parse_delta("-c-") },
            Edit { ei: 0, p: 3, u: 1, d: parse_delta("---d") },
            Assert(0, "acbd".to_owned()),
            Edit { ei: 1, p: 5, u: 1, d: parse_delta("-p-") },
            Assert(1, "apb".to_owned()),
            Edit { ei: 2, p: 4, u: 1, d: parse_delta("-r-") },
            Merge(0,2), Merge(1, 2),
            Assert(0, "acrbd".to_owned()),
            Assert(1, "arpb".to_owned()),
            Edit { ei: 1, p: 5, u: 1, d: parse_delta("----j") },
            Assert(1, "arpbj".to_owned()),
            Edit { ei: 2, p: 4, u: 1, d: parse_delta("---z") },
            Merge(0,2), Merge(1, 2),
            Assert(0, "acrbdz".to_owned()),
            Assert(1, "arpbzj".to_owned()),
            Merge(0,1),
            Assert(0, "acrpbdzj".to_owned()),
        ];
        MergeTestState::new(3).run_script(&script[..]);
    }

    /// Tests that merging again when there are no new revisions does nothing
    #[test]
    fn merge_idempotent() {
        use self::MergeTestOp::*;
        let script = vec![
            Edit { ei: 2, p: 1, u: 1, d: parse_delta("ab") },
            Merge(0,2), Merge(1, 2),
            Assert(0, "ab".to_owned()),
            Assert(1, "ab".to_owned()),
            Assert(2, "ab".to_owned()),
            Edit { ei: 0, p: 3, u: 1, d: parse_delta("-c-") },
            Edit { ei: 0, p: 3, u: 1, d: parse_delta("---d") },
            Assert(0, "acbd".to_owned()),
            Edit { ei: 1, p: 5, u: 1, d: parse_delta("-p-") },
            Edit { ei: 1, p: 5, u: 1, d: parse_delta("---j") },
            Merge(0,1),
            Assert(0, "acpbdj".to_owned()),
            Merge(0,1), Merge(1,0), Merge(0,1), Merge(1,0),
            Assert(0, "acpbdj".to_owned()),
            Assert(1, "acpbdj".to_owned()),
        ];
        MergeTestState::new(3).run_script(&script[..]);
    }

    #[test]
    fn merge_associative() {
        use self::MergeTestOp::*;
        let script = vec![
            Edit { ei: 2, p: 1, u: 1, d: parse_delta("ab") },
            Merge(0,2), Merge(1, 2),
            Edit { ei: 0, p: 3, u: 1, d: parse_delta("-c-") },
            Edit { ei: 1, p: 5, u: 1, d: parse_delta("-p-") },
            Edit { ei: 2, p: 2, u: 1, d: parse_delta("z--") },
            // copy the current state
            Merge(3, 0), Merge(4, 1), Merge(5, 2),
            // Do the merge one direction
            Merge(1,2),
            Merge(0,1),
            Assert(0, "zacpb".to_owned()),
            // Do it the other way on the copy
            Merge(4,3),
            Merge(5,4),
            Assert(5, "zacpb".to_owned()),
            // Go crazy
            Merge(0,5), Merge(2,5), Merge(4,5), Merge(1,4),
            Merge(3,1), Merge(5,3),
            AssertAll("zacpb".to_owned()),
        ];
        MergeTestState::new(6).run_script(&script[..]);
    }

    #[test]
    fn merge_simple_delete_1() {
        use self::MergeTestOp::*;
        let script = vec![
            Edit { ei: 0, p: 1, u: 1, d: parse_delta("abc") },
            Merge(1,0),
            Assert(0, "abc".to_owned()),
            Assert(1, "abc".to_owned()),
            Edit { ei: 0, p: 1, u: 1, d: parse_delta("!-d-") },
            Assert(0, "bdc".to_owned()),
            Edit { ei: 1, p: 3, u: 1, d: parse_delta("--efg!") },
            Assert(1, "abefg".to_owned()),
            Merge(1,0),
            Assert(1, "bdefg".to_owned()),
        ];
        MergeTestState::new(2).run_script(&script[..]);
    }

    #[test]
    fn merge_simple_delete_2() {
        use self::MergeTestOp::*;
        let script = vec![
            Edit { ei: 0, p: 1, u: 1, d: parse_delta("ab") },
            Merge(1,0),
            Assert(0, "ab".to_owned()),
            Assert(1, "ab".to_owned()),
            Edit { ei: 0, p: 1, u: 1, d: parse_delta("!-") },
            Assert(0, "b".to_owned()),
            Edit { ei: 1, p: 3, u: 1, d: parse_delta("-c-") },
            Assert(1, "acb".to_owned()),
            Merge(1,0),
            Assert(1, "cb".to_owned()),
        ];
        MergeTestState::new(2).run_script(&script[..]);
    }

    /// I have a scanned whiteboard diagram of doing this merge by hand, good for reference
    #[test]
    fn merge_whiteboard() {
        use self::MergeTestOp::*;
        let script = vec![
            Edit { ei: 2, p: 1, u: 1, d: parse_delta("ab") },
            Merge(0,2), Merge(1, 2), Merge(3, 2),
            Assert(0, "ab".to_owned()),
            Assert(1, "ab".to_owned()),
            Assert(2, "ab".to_owned()),
            Assert(3, "ab".to_owned()),
            Edit { ei: 2, p: 1, u: 1, d: parse_delta("!-") },
            Assert(2, "b".to_owned()),
            Edit { ei: 0, p: 3, u: 1, d: parse_delta("-c-") },
            Edit { ei: 0, p: 3, u: 1, d: parse_delta("---d") },
            Assert(0, "acbd".to_owned()),
            Merge(0,2),
            Assert(0, "cbd".to_owned()),
            Edit { ei: 1, p: 5, u: 1, d: parse_delta("-p-") },
            Merge(1,2),
            Assert(1, "pb".to_owned()),
            Edit { ei: 1, p: 5, u: 1, d: parse_delta("--j") },
            Assert(1, "pbj".to_owned()),
            // to replicate whiteboard, z must be before a tombstone
            // which we can do with another peer that inserts before a and merges.
            Edit { ei: 3, p: 7, u: 1, d: parse_delta("z--") },
            Merge(2,3),
            Merge(0,2), Merge(1, 2),
            Assert(0, "zcbd".to_owned()),
            Assert(1, "zpbj".to_owned()),
            Merge(0,1), // the merge from the whiteboard scan
            Assert(0, "zcpbdj".to_owned()),
        ];
        MergeTestState::new(4).run_script(&script[..]);
    }

    #[test]
    fn merge_max_undo_so_far() {
        use self::MergeTestOp::*;
        let script = vec![
            Edit { ei: 0, p: 1, u: 1, d: parse_delta("ab") },
            Merge(1,0), Merge(2,0),
            AssertMaxUndoSoFar(1,1),
            Edit { ei: 0, p: 1, u: 2, d: parse_delta("!-") },
            Edit { ei: 1, p: 3, u: 3, d: parse_delta("-!") },
            Merge(1,0),
            AssertMaxUndoSoFar(1,3),
            AssertMaxUndoSoFar(0,2),
            Merge(0,1),
            AssertMaxUndoSoFar(0,3),
            Edit { ei: 2, p: 1, u: 1, d: parse_delta("!!") },
            Merge(1,2),
            AssertMaxUndoSoFar(1,3),
        ];
        MergeTestState::new(3).run_script(&script[..]);
    }

    /// This is a regression test to ensure that session IDs are used to break
    /// ties in edit priorities. Otherwise the results may be inconsistent.
    #[test]
    fn merge_session_priorities() {
        use self::MergeTestOp::*;
        let script = vec![
            Edit { ei: 0, p: 1, u: 1, d: parse_delta("ac") },
            Merge(1,0),
            Merge(2,0),
            AssertAll("ac".to_owned()),
            Edit { ei: 0, p: 1, u: 1, d: parse_delta("-d-") },
            Assert(0, "adc".to_owned()),
            Edit { ei: 1, p: 1, u: 1, d: parse_delta("-f-") },
            Merge(2,1),
            Assert(1, "afc".to_owned()),
            Assert(2, "afc".to_owned()),
            Merge(2,0),
            Merge(0,1),
            // These two will be different without using session IDs
            Assert(2, "adfc".to_owned()),
            Assert(0, "adfc".to_owned()),
        ];
        MergeTestState::new(3).run_script(&script[..]);
    }
}
