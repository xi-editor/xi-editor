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

//! An engine for handling edits (possibly from async sources) and undo. This
//! module actually implements a mini Conflict-free Replicated Data Type, but
//! is considerably simpler than the usual CRDT implementation techniques,
//! because all operations are serialized in this central engine.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std;

use rope::{Rope, RopeInfo};
use subset::Subset;
use delta::Delta;

pub struct Engine {
    rev_id_counter: usize,
    union_str: Rope,
    revs: Vec<Revision>,
}

struct Revision {
    rev_id: usize,
    deletes_from_union: Subset,
    union_str_len: usize,
    edit: Contents,
}

use self::Contents::*;

enum Contents {
    Edit {
        priority: usize,
        undo_group: usize,
        inserts: Subset,
        deletes: Subset,
    },
    Undo {
        groups: BTreeSet<usize>,  // set of undo_group id's
    }
}

impl Engine {
    pub fn new(initial_contents: Rope) -> Engine {
        let rev = Revision {
            rev_id: 0,
            deletes_from_union: Subset::default(),
            union_str_len: initial_contents.len(),
            edit: Undo { groups: BTreeSet::default() },
        };
        Engine {
            rev_id_counter: 1,
            union_str: initial_contents,
            revs: vec![rev],
        }
    }

    fn get_current_undo(&self) -> Option<&BTreeSet<usize>> {
        for rev in self.revs.iter().rev() {
            if let Undo { ref groups } = rev.edit {
                return Some(groups);
            }
        }
        None
    }

    fn find_rev(&self, rev_id: usize) -> Option<usize> {
        for (i, rev) in self.revs.iter().enumerate().rev() {
            if rev.rev_id == rev_id {
                return Some(i)
            }
        }
        None
    }

    /// Get the contents of the document at a given revision number
    fn rev_content_for_index(&self, rev_index: usize) -> Rope {
        self.deletes_from_union_for_index(rev_index).delete_from(&self.union_str)
    }

    /// Get the Subset to delete from the current union string in order to obtain a revision's content
    fn deletes_from_union_for_index(&self, rev_index: usize) -> Cow<Subset> {
        let mut deletes_from_union = Cow::Borrowed(&self.revs[rev_index].deletes_from_union);
        for rev in &self.revs[rev_index + 1..] {
            if let Edit { ref inserts, .. } = rev.edit {
                if !inserts.is_empty() {
                    deletes_from_union = Cow::Owned(deletes_from_union.transform_union(inserts));
                }
            }
        }
        deletes_from_union
    }

    /// Get revision id of head revision.
    pub fn get_head_rev_id(&self) -> usize {
        self.revs.last().unwrap().rev_id
    }

    /// Get text of head revision.
    pub fn get_head(&self) -> Rope {
        self.rev_content_for_index(self.revs.len() - 1)
    }

    /// Get text of a given revision, if it can be found.
    pub fn get_rev(&self, rev: usize) -> Option<Rope> {
        self.find_rev(rev).map(|rev_index| self.rev_content_for_index(rev_index))
    }

    /// A delta that, when applied to previous head, results in the current head. Panics
    /// if there is not at least one edit.
    pub fn delta_rev_head(&self, base_rev: usize) -> Delta<RopeInfo> {
        let ix = self.find_rev(base_rev).expect("base revision not found");
        let rev = &self.revs[ix];
        let mut prev_from_union = Cow::Borrowed(&rev.deletes_from_union);
        for r in &self.revs[ix + 1..] {
            if let Edit { ref inserts, .. } = r.edit {
                if !inserts.is_empty() {
                    prev_from_union = Cow::Owned(prev_from_union.transform_union(inserts));
                }
            }
        }
        let head_rev = &self.revs.last().unwrap();
        Delta::synthesize(&self.union_str, &prev_from_union, &head_rev.deletes_from_union)
    }

    fn mk_new_rev(&self, new_priority: usize, undo_group: usize,
            base_rev: usize, delta: Delta<RopeInfo>) -> (Revision, Rope) {
        let ix = self.find_rev(base_rev).expect("base revision not found");
        let rev = &self.revs[ix];
        let (ins_delta, deletes) = delta.factor();
        let mut union_ins_delta = ins_delta.transform_expand(&rev.deletes_from_union, rev.union_str_len, true);
        let mut new_deletes = deletes.transform_expand(&rev.deletes_from_union);
        for r in &self.revs[ix + 1..] {
            if let Edit { priority, ref inserts, .. } = r.edit {
                if !inserts.is_empty() {
                    let after = new_priority >= priority;  // should never be ==
                    union_ins_delta = union_ins_delta.transform_expand(inserts, r.union_str_len, after);
                    new_deletes = new_deletes.transform_expand(inserts);
                }
            }
        }
        let new_inserts = union_ins_delta.inserted_subset();
        if !new_inserts.is_empty() {
            new_deletes = new_deletes.transform_expand(&new_inserts);
        }
        let new_union_str = union_ins_delta.apply(&self.union_str);
        let undone = self.get_current_undo().map_or(false, |undos| undos.contains(&undo_group));
        let mut new_from_union = Cow::Borrowed(&self.revs.last().unwrap().deletes_from_union);
        if undone {
            if !new_inserts.is_empty() {
                new_from_union = Cow::Owned(new_from_union.transform_union(&new_inserts));
            }
        } else {
            if !new_inserts.is_empty() {
                new_from_union = Cow::Owned(new_from_union.transform_expand(&new_inserts));
            }
            if !new_deletes.is_empty() {
                new_from_union = Cow::Owned(new_from_union.union(&new_deletes));
            }
        }
        (Revision {
            rev_id: self.rev_id_counter,
            deletes_from_union: new_from_union.into_owned(),
            union_str_len: new_union_str.len(),
            edit: Edit {
                priority: new_priority,
                undo_group: undo_group,
                inserts: new_inserts,
                deletes: new_deletes,
            }
        }, new_union_str)
    }

    pub fn edit_rev(&mut self, priority: usize, undo_group: usize,
            base_rev: usize, delta: Delta<RopeInfo>) {
        let (new_rev, new_union_str) = self.mk_new_rev(priority, undo_group, base_rev, delta);
        self.rev_id_counter += 1;
        self.revs.push(new_rev);
        self.union_str = new_union_str;
    }

    // This computes undo all the way from the beginning. An optimization would be to not
    // recompute the prefix up to where the history diverges, but it's not clear that's
    // even worth the code complexity.
    fn compute_undo(&self, groups: BTreeSet<usize>) -> Revision {
        let mut deletes_from_union = Subset::default();
        for rev in &self.revs {
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
        Revision {
            rev_id: self.rev_id_counter,
            deletes_from_union: deletes_from_union,
            union_str_len: self.union_str.len(),
            edit: Undo {
                groups: groups
            }
        }
    }

    pub fn undo(&mut self, groups: BTreeSet<usize>) {
        let new_rev = self.compute_undo(groups);
        self.revs.push(new_rev);
        self.rev_id_counter += 1;
    }

    pub fn is_equivalent_revision(&self, base_rev: usize, other_rev: usize) -> bool {
        let base_subset = self.find_rev(base_rev).map(|rev_index| self.deletes_from_union_for_index(rev_index));
        let other_subset = self.find_rev(other_rev).map(|rev_index| self.deletes_from_union_for_index(rev_index));

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
        let mut gc_dels = Subset::default();
        // TODO: want to let caller retain more rev_id's.
        let mut retain_revs = BTreeSet::new();
        if let Some(last) = self.revs.last() {
            retain_revs.insert(last.rev_id);
        }
        {
            let cur_undo = self.get_current_undo();
            for rev in &self.revs {
                if let Edit { ref undo_group, ref inserts, ref deletes, .. } = rev.edit {
                    if !retain_revs.contains(&rev.rev_id) && gc_groups.contains(undo_group) {
                        if cur_undo.map_or(false, |undos| undos.contains(undo_group)) {
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
            self.union_str = gc_dels.delete_from(&self.union_str);
        }
        let old_revs = std::mem::replace(&mut self.revs, Vec::new());
        for rev in old_revs.into_iter().rev() {
            match rev.edit {
                Edit { priority, undo_group, inserts, deletes } => {
                    let new_gc_dels = if inserts.is_empty() {
                        None
                    } else {
                        Some(inserts.transform_shrink(&gc_dels))
                    };
                    if retain_revs.contains(&rev.rev_id) || !gc_groups.contains(&undo_group) {
                        let (inserts, deletes, deletes_from_union, len) = if gc_dels.is_empty() {
                            (inserts, deletes, rev.deletes_from_union, rev.union_str_len)
                        } else {
                            (gc_dels.transform_shrink(&inserts),
                                gc_dels.transform_shrink(&deletes),
                                gc_dels.transform_shrink(&rev.deletes_from_union),
                                gc_dels.len_after_delete(rev.union_str_len))
                        };
                        self.revs.push(Revision {
                            rev_id: rev.rev_id,
                            deletes_from_union: deletes_from_union,
                            union_str_len: len,
                            edit: Edit {
                                priority: priority,
                                undo_group: undo_group,
                                inserts: inserts,
                                deletes: deletes,
                            }
                        });
                    }
                    if let Some(new_gc_dels) = new_gc_dels {
                        gc_dels = new_gc_dels;
                    }
                }
                Undo { groups } => {
                    // We're super-aggressive about dropping these; after gc, the history
                    // of which undos were used to compute deletes_from_union in edits may be lost.
                    if retain_revs.contains(&rev.rev_id) {
                        let (deletes_from_union, len) = if gc_dels.is_empty() {
                            (rev.deletes_from_union, rev.union_str_len)
                        } else {
                            (gc_dels.transform_shrink(&rev.deletes_from_union),
                                gc_dels.len_after_delete(rev.union_str_len))
                        };
                        self.revs.push(Revision {
                            rev_id: rev.rev_id,
                            deletes_from_union: deletes_from_union,
                            union_str_len: len,
                            edit: Undo {
                                groups: &groups - gc_groups,
                            }
                        })
                    }
                }
            }
        }
        self.revs.reverse();
    }
}
