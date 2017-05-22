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
use multiset::{Subset, CountMatcher};
use delta::Delta;

#[derive(Debug)]
pub struct Engine {
    rev_id_counter: usize,
    text: Rope,
    tombstones: Rope,
    deletes_from_union: Subset,
    revs: Vec<Revision>,
}

#[derive(Debug)]
struct Revision {
    rev_id: usize,
    deletes_from_union: Subset,
    edit: Contents,
}

use self::Contents::*;

#[derive(Debug)]
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
        let deletes_from_union = Subset::new(initial_contents.len());
        let rev = Revision {
            rev_id: 0,
            deletes_from_union: deletes_from_union.clone(),
            edit: Undo { groups: BTreeSet::default() },
        };
        Engine {
            rev_id_counter: 1,
            text: initial_contents,
            tombstones: Rope::default(),
            deletes_from_union,
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
        let old_deletes_from_union = self.deletes_from_union_for_index(rev_index);
        let delta = Delta::synthesize(&self.tombstones,
            &self.deletes_from_union, &old_deletes_from_union);
        delta.apply(&self.text)
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
    pub fn get_head(&self) -> &Rope {
        &self.text
    }

    /// Get text of a given revision, if it can be found.
    pub fn get_rev(&self, rev: usize) -> Option<Rope> {
        self.find_rev(rev).map(|rev_index| self.rev_content_for_index(rev_index))
    }

    /// A delta that, when applied to `base_rev`, results in the current head. Panics
    /// if there is not at least one edit.
    pub fn delta_rev_head(&self, base_rev: usize) -> Delta<RopeInfo> {
        let ix = self.find_rev(base_rev).expect("base revision not found");
        let rev = &self.revs[ix];

        // Delta::synthesize will add inserts for everything that is in
        // prev_from_union (old deletes) but not in
        // head_rev.deletes_from_union (new deletes). So we add all inserts
        // since base_rev to prev_from_union so that they will be inserted in
        // the Delta if they weren't also deleted.
        let mut prev_from_union = Cow::Borrowed(&rev.deletes_from_union);
        for r in &self.revs[ix + 1..] {
            if let Edit { ref inserts, .. } = r.edit {
                if !inserts.is_empty() {
                    prev_from_union = Cow::Owned(prev_from_union.transform_union(inserts));
                }
            }
        }

        // TODO: this does 2 calls to Delta::synthesize and 1 to apply, this probably could be better.
        let old_tombstones = Engine::shuffle_tombstones(&self.text, &self.tombstones, &self.deletes_from_union, &prev_from_union);
        Delta::synthesize(&old_tombstones, &prev_from_union, &self.deletes_from_union)
    }

    // TODO: don't construct transform if subsets are empty
    /// Retuns a tuple of a new `Revision` representing the edit based on the
    /// current head, a new text `Rope`, and a new tombstones `Rope`.
    fn mk_new_rev(&self, new_priority: usize, undo_group: usize,
            base_rev: usize, delta: Delta<RopeInfo>) -> (Revision, Rope, Rope, Subset) {
        let ix = self.find_rev(base_rev).expect("base revision not found");
        let rev = &self.revs[ix];
        let (ins_delta, deletes) = delta.factor();

        // rebase delta to be on the base_rev union instead of the text
        let mut union_ins_delta = ins_delta.transform_expand(&rev.deletes_from_union, true);
        let mut new_deletes = deletes.transform_expand(&rev.deletes_from_union);

        // rebase the delta to be on the head union instead of the base_rev union
        for r in &self.revs[ix + 1..] {
            if let Edit { priority, ref inserts, .. } = r.edit {
                if !inserts.is_empty() {
                    let after = new_priority >= priority;  // should never be ==
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
        let undone = self.get_current_undo().map_or(false, |undos| undos.contains(&undo_group));
        let new_deletes_from_union = {
            let to_delete = if undone { &new_inserts } else { &new_deletes };
            rebased_deletes_from_union.union(to_delete)
        };

        // move deleted or undone-inserted things from text to tombstones
        let (new_text, new_tombstones) = Engine::shuffle(&text_with_inserts, &self.tombstones,
            &rebased_deletes_from_union, &new_deletes_from_union);

        (Revision {
            rev_id: self.rev_id_counter,
            deletes_from_union: new_deletes_from_union.clone(),
            edit: Edit {
                priority: new_priority,
                undo_group: undo_group,
                inserts: new_inserts,
                deletes: new_deletes,
            }
        }, new_text, new_tombstones, new_deletes_from_union)
    }

    /// Move sections from text to tombstones and out of tombstones based on a new and old set of deletions
    fn shuffle_tombstones(text: &Rope, tombstones: &Rope,
            old_deletes_from_union: &Subset, new_deletes_from_union: &Subset) -> Rope {
        // Taking the complement of deletes_from_union leads to an interleaving valid for swapped text and tombstones,
        // allowing us to use the same method to insert the text into the tombstones.
        let inverse_tombstones_map = old_deletes_from_union.complement();
        let move_delta = Delta::synthesize(text, &inverse_tombstones_map, &new_deletes_from_union.complement());
        move_delta.apply(tombstones)
    }

    /// Move sections from text to tombstones and vice versa based on a new and old set of deletions.
    /// Returns a tuple of a new text `Rope` and a new `Tombstones` rope described by `new_deletes_from_union`.
    fn shuffle(text: &Rope, tombstones: &Rope,
            old_deletes_from_union: &Subset, new_deletes_from_union: &Subset) -> (Rope,Rope) {
        // Delta that deletes the right bits from the text
        let del_delta = Delta::synthesize(tombstones, old_deletes_from_union, new_deletes_from_union);
        let new_text = del_delta.apply(text);
        // println!("shuffle: old={:?} new={:?} old_text={:?} new_text={:?} old_tombstones={:?}",
        //     old_deletes_from_union, new_deletes_from_union, text, new_text, tombstones);
        (new_text, Engine::shuffle_tombstones(text,tombstones,old_deletes_from_union,new_deletes_from_union))
    }

    pub fn edit_rev(&mut self, priority: usize, undo_group: usize,
            base_rev: usize, delta: Delta<RopeInfo>) {
        let (new_rev, new_text, new_tombstones, new_deletes_from_union) =
            self.mk_new_rev(priority, undo_group, base_rev, delta);
        self.rev_id_counter += 1;
        self.revs.push(new_rev);
        self.text = new_text;
        self.tombstones = new_tombstones;
        self.deletes_from_union = new_deletes_from_union;
    }

    // since undo and gc replay history with transforms, we need an empty set
    // of the union string length *before* the first revision.
    fn empty_subset_before_first_rev(&self) -> Subset {
        let first_rev = &self.revs.first().unwrap();
        // it will be immediately transform_expanded by inserts if it is an Edit, so length must be before
        let len = match first_rev.edit {
            Edit { ref inserts, .. } => inserts.count(CountMatcher::Zero),
            // TODO: replace this with count of flipped_deletes in Undo case
            Undo { .. } => first_rev.deletes_from_union.count(CountMatcher::All),
        };
        Subset::new(len)
    }

    // This computes undo all the way from the beginning. An optimization would be to not
    // recompute the prefix up to where the history diverges, but it's not clear that's
    // even worth the code complexity.
    fn compute_undo(&self, groups: BTreeSet<usize>) -> (Revision, Subset) {
        let mut deletes_from_union = self.empty_subset_before_first_rev();
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
        (Revision {
            rev_id: self.rev_id_counter,
            deletes_from_union: deletes_from_union.clone(),
            edit: Undo {
                groups: groups
            }
        }, deletes_from_union)
    }

    pub fn undo(&mut self, groups: BTreeSet<usize>) {
        let (new_rev, new_deletes_from_union) = self.compute_undo(groups);

        let (new_text, new_tombstones) =
            Engine::shuffle(&self.text, &self.tombstones, &self.deletes_from_union, &new_deletes_from_union);

        self.text = new_text;
        self.tombstones = new_tombstones;
        self.deletes_from_union = new_deletes_from_union;
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
        let mut gc_dels = self.empty_subset_before_first_rev();
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
            let head_rev = &self.revs.last().unwrap();
            let not_in_tombstones = head_rev.deletes_from_union.complement();
            let dels_from_tombstones = gc_dels.transform_shrink(&not_in_tombstones);
            self.tombstones = dels_from_tombstones.delete_from(&self.tombstones);
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
                        let (inserts, deletes, deletes_from_union) = if gc_dels.is_empty() {
                            (inserts, deletes, rev.deletes_from_union)
                        } else {
                            (inserts.transform_shrink(&gc_dels),
                                deletes.transform_shrink(&gc_dels),
                                rev.deletes_from_union.transform_shrink(&gc_dels))
                        };
                        self.revs.push(Revision {
                            rev_id: rev.rev_id,
                            deletes_from_union: deletes_from_union,
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
                        let deletes_from_union = if gc_dels.is_empty() {
                            rev.deletes_from_union
                        } else {
                            rev.deletes_from_union.transform_shrink(&gc_dels)
                        };
                        self.revs.push(Revision {
                            rev_id: rev.rev_id,
                            deletes_from_union: deletes_from_union,
                            edit: Undo {
                                groups: &groups - gc_groups,
                            }
                        })
                    }
                }
            }
        }
        self.revs.reverse();
        self.deletes_from_union = self.revs.last().unwrap().deletes_from_union.clone();
    }
}

#[cfg(test)]
mod tests {
    use engine::Engine;
    use rope::{Rope, RopeInfo};
    use delta::{Builder, Delta};
    use interval::Interval;
    use std::collections::BTreeSet;

    const TEST_STR: &'static str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

    fn build_delta_1() -> Delta<RopeInfo> {
        let mut d_builder = Builder::new(TEST_STR.len());
        d_builder.delete(Interval::new_closed_open(10, 36));
        d_builder.replace(Interval::new_closed_open(39, 42), Rope::from("DEEF"));
        d_builder.replace(Interval::new_closed_open(54, 54), Rope::from("999"));
        d_builder.delete(Interval::new_closed_open(58, 61));
        d_builder.build()
    }

    fn build_delta_2() -> Delta<RopeInfo> {
        let mut d_builder = Builder::new(TEST_STR.len());
        d_builder.replace(Interval::new_closed_open(1, 3), Rope::from("!"));
        d_builder.delete(Interval::new_closed_open(10, 36));
        d_builder.replace(Interval::new_closed_open(42, 45), Rope::from("GI"));
        d_builder.replace(Interval::new_closed_open(54, 54), Rope::from("888"));
        d_builder.replace(Interval::new_closed_open(59, 60), Rope::from("HI"));
        d_builder.build()
    }

    #[test]
    fn edit_rev_simple() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        engine.edit_rev(0, 0, 0, build_delta_1());
        assert_eq!("0123456789abcDEEFghijklmnopqr999stuvz", String::from(engine.get_head()));
    }

    #[test]
    fn edit_rev_concurrent() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        engine.edit_rev(1, 0, 0, build_delta_1());
        engine.edit_rev(0, 1, 0, build_delta_2());
        assert_eq!("0!3456789abcDEEFGIjklmnopqr888999stuvHIz", String::from(engine.get_head()));
    }

    fn undo_test(before: bool, undos : BTreeSet<usize>, output: &str) {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        if before {
            engine.undo(undos.clone());
        }
        engine.edit_rev(1, 0, 0, build_delta_1());
        engine.edit_rev(0, 1, 0, build_delta_2());
        if !before {
            engine.undo(undos);
        }
        assert_eq!(output, String::from(engine.get_head()));
    }

    #[test]
    fn edit_rev_undo() {
        undo_test(true, [0,1].iter().cloned().collect(), TEST_STR);
    }

    #[test]
    fn edit_rev_undo_2() {
        undo_test(true, [1].iter().cloned().collect(), "0123456789abcDEEFghijklmnopqr999stuvz");
    }

    #[test]
    fn edit_rev_undo_3() {
        undo_test(true, [0].iter().cloned().collect(), "0!3456789abcdefGIjklmnopqr888stuvwHIyz");
    }

    #[test]
    fn delta_rev_head() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        engine.edit_rev(1, 0, 0, build_delta_1());
        let d = engine.delta_rev_head(0);
        assert_eq!(String::from(engine.get_head()), d.apply_to_string(TEST_STR));
    }

    #[test]
    fn delta_rev_head_2() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        engine.edit_rev(1, 0, 0, build_delta_1());
        engine.edit_rev(0, 1, 0, build_delta_2());
        let d = engine.delta_rev_head(0);
        assert_eq!(String::from(engine.get_head()), d.apply_to_string(TEST_STR));
    }

    #[test]
    fn delta_rev_head_3() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        engine.edit_rev(1, 0, 0, build_delta_1());
        engine.edit_rev(0, 1, 0, build_delta_2());
        let d = engine.delta_rev_head(1);
        assert_eq!(String::from(engine.get_head()), d.apply_to_string("0123456789abcDEEFghijklmnopqr999stuvz"));
    }

    #[test]
    fn undo() {
        undo_test(false, [0,1].iter().cloned().collect(), TEST_STR);
    }

    #[test]
    fn undo_2() {
        undo_test(false, [1].iter().cloned().collect(), "0123456789abcDEEFghijklmnopqr999stuvz");
    }

    #[test]
    fn undo_3() {
        undo_test(false, [0].iter().cloned().collect(), "0!3456789abcdefGIjklmnopqr888stuvwHIyz");
    }

    #[test]
    fn undo_4() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        let d1 = Delta::simple_edit(Interval::new_closed_open(0,0), Rope::from("a"), TEST_STR.len());
        engine.edit_rev(1, 0, 0, d1.clone());
        engine.undo([0].iter().cloned().collect());
        let d2 = Delta::simple_edit(Interval::new_closed_open(0,0), Rope::from("a"), TEST_STR.len()+1);
        engine.edit_rev(1, 1, 1, d2); // note this is based on d1 before, not the undo
        let new_head = engine.get_head_rev_id();
        let d3 = Delta::simple_edit(Interval::new_closed_open(0,0), Rope::from("b"), TEST_STR.len()+1);
        engine.edit_rev(1, 2, new_head, d3);
        engine.undo([0,2].iter().cloned().collect());
        assert_eq!("a0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz", String::from(engine.get_head()));
    }

    #[test]
    fn undo_5() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        let d1 = Delta::simple_edit(Interval::new_closed_open(0,10), Rope::from(""), TEST_STR.len());
        engine.edit_rev(1, 0, 0, d1.clone());
        engine.edit_rev(1, 1, 0, d1.clone());
        engine.undo([0].iter().cloned().collect());
        assert_eq!("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz", String::from(engine.get_head()));
        engine.undo([0,1].iter().cloned().collect());
        assert_eq!(TEST_STR, String::from(engine.get_head()));
        engine.undo([].iter().cloned().collect());
        assert_eq!("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz", String::from(engine.get_head()));
    }

    #[test]
    fn gc() {
        let mut engine = Engine::new(Rope::from(TEST_STR));
        let d1 = Delta::simple_edit(Interval::new_closed_open(0,0), Rope::from("c"), TEST_STR.len());
        engine.edit_rev(1, 0, 0, d1);
        engine.undo([0].iter().cloned().collect());
        let d2 = Delta::simple_edit(Interval::new_closed_open(0,0), Rope::from("a"), TEST_STR.len()+1);
        engine.edit_rev(1, 1, 1, d2);
        let gc : BTreeSet<usize> = [0].iter().cloned().collect();
        engine.gc(&gc);
        let d3 = Delta::simple_edit(Interval::new_closed_open(0,0), Rope::from("b"), TEST_STR.len()+1);
        let new_head = engine.get_head_rev_id();
        engine.edit_rev(1, 2, new_head, d3);
        engine.undo([2].iter().cloned().collect());
        assert_eq!("a0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz", String::from(engine.get_head()));
    }

    /// This case is a regression test reproducing a panic I found while using the UI.
    /// It does undos and gcs in a pattern that can actually happen when using the editor.
    fn gc_scenario(edits: usize, max_undos: usize) {
        let mut engine = Engine::new(Rope::from(""));

        // insert `edits` letter "b"s in separate undo groups
        for i in 0..edits {
            let d = Delta::simple_edit(Interval::new_closed_open(0,0), Rope::from("b"), i);
            let head = engine.get_head_rev_id();
            engine.edit_rev(1, i, head, d);
            if i >= max_undos {
                let to_gc : BTreeSet<usize> = [i-max_undos].iter().cloned().collect();
                engine.gc(&to_gc)
            }
        }

        // spam cmd+z until the available undo history is exhausted
        let mut to_undo = BTreeSet::new();
        for i in ((edits-max_undos)..edits).rev() {
            to_undo.insert(i);
            engine.undo(to_undo.clone());
        }

        // insert a character at the beginning
        let d1 = Delta::simple_edit(Interval::new_closed_open(0,0), Rope::from("h"), engine.get_head().len());
        let head = engine.get_head_rev_id();
        engine.edit_rev(1, edits, head, d1);

        // since character was inserted after gc, editor gcs all undone things
        engine.gc(&to_undo);

        // insert character at end, when this test was added, it panic'd here
        let chars_left = (edits-max_undos)+1;
        let d2 = Delta::simple_edit(Interval::new_closed_open(chars_left, chars_left), Rope::from("f"), engine.get_head().len());
        let head2 = engine.get_head_rev_id();
        engine.edit_rev(1, edits, head2, d2);

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
        let d1 = Delta::simple_edit(Interval::new_closed_open(0,10), Rope::from(""), TEST_STR.len());
        engine.edit_rev(1, 0, 0, d1.clone());
        engine.edit_rev(1, 1, 0, d1.clone());
        let gc : BTreeSet<usize> = [0].iter().cloned().collect();
        engine.gc(&gc);
        // shouldn't do anything since it was double-deleted and one was GC'd
        engine.undo([0,1].iter().cloned().collect());
        assert_eq!("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz", String::from(engine.get_head()));
    }
}
