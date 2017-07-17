# The Xi Text Engine CRDT

This document contains a detailed description of Xi's text Conflict-free Replicated Data Type (CRDT) as implemented in the `xi-rope` crate. If you want an overview of the motivation behind using a CRDT and a conceptual description of what the CRDT does see `crdt.md`.

## Table of Contents

- [Representation](#representation): Describes the representation Xi uses to implement the CRDT in a memory and time efficient way.
- [Operations](#operations): Describes all the operations implemented on the representation to allow it to support undo, asynchronous edits, distributed synchronization and more.

## Representation

The conceptual representation described in `crdt.md` would be very inefficient to use directly. If we had to store an ID and ordering edges for each character and reconstruct the current text via topological sort every time we wanted to know what the current text, Xi would be incredibly slow and use tons of memory.

Instead, we use a representation that allows all the operations we care about to be fast. We also take advantaged of the typical patterns of text document usage to make the representation more memory efficient for common cases.

The key optimization that shapes everything else is to avoid using IDs for characters or storing ordering edges explicitly. Instead, we represent the identity of characters implicitly by their position in the current text. But then how do we reference them in our revision history? We could rewrite all the indices in the history every time we made an edit, but that would be terribly inefficient. Instead the set of inserted characters in every revision is treated as a *coordinate transform* for the older revisions. In order to find the character referred to by an older revision you have to transform the indices it uses based on the insertions made after it.

That description is almost certainly too vague to be understandable at this point, but don't worry, there will be a full description with diagrams later in the document.

Starting from the basic building blocks and proceeding towards the top level CRDT `Engine`, here are all the structures:

### Rope

```rust
pub struct Rope(Arc<RopeBody>);

#[derive(Clone)]
struct RopeBody {
    /// Used for efficiently seeking to an index in the tree
    len: usize,
    /// Used for rebalancing
    height: usize,
    val: RopeVal,
}

enum RopeVal {
    Leaf(String),
    Internal(Vec<Rope>),
}
```
**Note:** All the Rust code in this document is simplified from the actual implementation so that it still conveys the structure and memory properties of the representation but elides details not necessary to understand the CRDT. See the code or generated docs for the full definitions. For example, the real struct for `Rope` is called `Node<N: NodeInfo>` and is a generic structure not specific to text, that is later instantiated for text as `pub type Rope = Node<RopeInfo>`.

When representing potentially large amounts of text, Xi avoids using `String`s and instead uses a data structure called `Rope`. This is essentially an immutable `String` except many operations that would be `O(n)` with normal strings are instead `O(log n)` or `O(1)`. Some examples of operations like this:

- Copying
- Extracting a substring by index
- Inserting one piece of text in the middle of another producing a new piece of text
- Deleting an interval from a piece of text, producing a new piece of text

Behind the scenes, `Rope` is an immutable balanced tree structure using Rust's atomic reference counting smart pointer (`Arc`) to share data, so "copying" any sub-tree is a very fast `O(1)` operation. The leaves of the tree are chunks of text with a maximum size. For example, deleting an interval of a `Rope` only requires creating a few new nodes that reference the sub-tree to the right of the deleted interval and to the left, and creating up to two new leaves if the deleted interval doesn't lie on a chunk boundary.

Obviously `Rope`s will be slower and take more memory than small `Strings` but they have an asymptotic advantage when working with large documents.

For a deeper look at `Rope`s see the [Rope Science](rope_science/intro.md) series.

### Subset

```rust
struct Segment {
    len: usize,
    count: usize,
}

pub struct Subset {
    /// Invariant, maintained by `SubsetBuilder`: all `Segment`s have non-zero
    /// length, and no `Segment` has the same count as the one before it.
    segments: Vec<Segment>,
}
```

The `Subset` structure in `multiset.rs` represents a multi-subset of a string, meaning that every character in the string has a count (often `0`) representing how many times it is in the `Subset`. Most of the time this structure is used to represent plain-old subsets and the counts are only ever `0` for something not in the set or `1` for a character in the set.

It stores this information compactly as a list of consecutive `Segment`s with a `length` and a `count`. This way a `Subset` representing 1000 consecutive characters in the middle of a string will only require 3 segments (a 0-count one at the start, a 1-count one in the middle, and another 0-count one at the end).

The primary reason that `Subset`s can have counts greater than `1` is to represent concurrent deletes, for example if two concurrent edits delete the same character, and one of them is undone, subtracting one of the deletes from the `Subset` of deleted characters should still leave the character deleted once. For this reason, the `Subset`s of deleted characters which are described later have counts that represent how many times each character has been deleted.

Note that an "empty" `Subset` where all the characters have count `0` is still represented as a single segment with the length of the base string and count set to `0`. This allows functions using `Subset`s to panic if they are used with strings or other `Subset`s of the wrong length. This gives a level of dynamic checking that algorithms are using `Subset`s correctly.

### Delta

```rust
enum DeltaElement {
    /// Represents a range of text in the base document. Includes beginning, excludes end.
    Copy(usize, usize),
    Insert(Rope),
}

pub struct Delta {
    els: Vec<DeltaElement>,
    /// The total length of the base document, used for checks in some operations
    base_len: usize,
}

pub struct InsertDelta(Delta);
```

Delta represents the difference between one string (*A*) and another (*B*). It stores this as a list of intervals copied from the *A* string and new inserted sections. All the indices of the copied intervals are non-decreasing. So a deletion is represented as a section of the *A* string which isn't copied to the *B* string, and insertions are represented as inserted sections in between copied sections.

There is also a type `InsertDelta` that is just a wrapper around a `Delta` but represents a guarantee that the `Delta` only inserts, that is, the entire *A* string is copied by the `Copy` intervals.

### The "union string"

A super important part of being able to provide the properties we desire in our CRDT is that we never throw useful information away. This means that when you delete text in Xi, or undo an insert, the text doesn't actually get thrown away, just marked as deleted. You can think of this as if there is a "union string" that contains all the characters that have ever been inserted. When we delete or undo, we don't touch the union string, we just change a `Subset` (`deletes_from_union`, more on that later) which marks which characters of the union string are deleted. These deleted characters are sometimes called "tombstones" both within Xi and the academic CRDT literature.

If we wanted to go from the "union string" to the current text of the document, you'd delete the characters marked in `deletes_from_union` from the union string. The problem is you want to get the current text really often, so this would be an inefficient way to actually store the text.

Thus, the "union string" isn't really a structure in Xi, although it used to be, it's just a concept that could theoretically be created from the information we do have. However, the union string is still used for most indices as a coordinate space for `Subset`s. You'll see later how we actually store the current text, but you'll notice that most associated data still has its indices based on the union string.

### Revision

```rust
struct Revision {
    /// This uniquely represents the identity of this revision and it stays
    /// the same even it is rebased or merged between devices.
    rev_id: RevId,
    /// The largest undo group number of any edit in the history up to this
    /// point. Used to optimize undo to not look further back.
    max_undo_so_far: usize,
    edit: Contents,
}

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
        toggled_groups: BTreeSet<usize>,  // set of undo_group id's
        /// Used to store a reversible difference between the deleted
        /// characters before and after this operation.
        deletes_bitxor: Subset,
    }
}
```

`Revision`s represent a single edit to the document. Typing a character into a Xi document will create a new `Revision` with a `Contents::Edit` with an empty `deletes` subset and an `inserts` subset containing the character inserted.

They can also represent more complex things like selecting multiple ranges of text using multiple cursors (which Xi supports) and then pasting. This would result in a `Contents::Edit` with an `inserts` subset containing multiple separate segments of pasted characters, and a `deletes` subset containing the multiple ranges of previous text that were replaced.

Note that the `inserts` and `deletes` `Subset`s are based on the union string described above, this allows insertions and deletions to maintain their position easily in the face of concurrency and undo. For example, say I have the text "ac" and I change it to "abc", but then undo the first edit leaving "b". If I re-do the first edit, Xi needs to know that the "b" goes between the two deleted characters. You might be able to think of ways to do this with other coordinates, but it's much easier and less fraught when coordinates only change on insertions instead of insertions, deletions and undo.

A key property of `Revision`s is that they contain all the necessary information to apply them as well as reverse them. This is important both for undo and also for some CRDT operations we'll get to later. This is why `Contents::undo` stores the set of toggled groups rather than the new set of undone groups. It's also as why it stores a reversible set of changes to the deleted characters (more on those later), this could be found by replaying all of history using the new set of undo groups, but then it would be inefficient to apply and reverse (because it would be proportional to the length of history).

### RevId & RevToken

```rust
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

/// Valid within a session. If there's a collision the most recent matching
/// commit will be used, which means only the (small) set of concurrent edits
/// could trigger incorrect behavior if they collide, so u64 is safe.
pub type RevToken = u64;
```

`RevId` is used to uniquely identify the revision. The trick is offline devices have to be able to generate non-colliding IDs, which they do by generating random "session IDs" that become part of their revision numbers for that execution, with `num` being just an incrementing counter. The reason the IDs aren't fully random is so that eventually we can delta-compress them and the IDs will take on average 1 bit per revision instead of 128 bits. This is only necessary for the multi-device syncing case, in the single-device case the session ID is always `(1,0)`.

`RevToken` is used to make the API simpler, it is just the hash of a `RevId`. This makes things easy for plugins and other things that need to reference revisions.


### Engine

```rust
/// Represents the current state of a document and all of its history
pub struct Engine {
    /// The session ID used to create new `RevId`s for edits made on this device
    session: (u64, u32),
    /// The incrementing revision number counter for this session used for `RevId`s
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
    undone_groups: BTreeSet<usize>,  // set of undo_group id's
    /// The revision history of the document
    revs: Vec<Revision>,
}
```

<!-- TODO diagram of text tombstones and union string -->

`Engine` is the top-level container of text state for the CRDT. It stores the current state of the document and all the `Revision`s that lead up to it. This allows operations that require knowledge of history to walk apply `Revision`s in reverse from the current state to find the state at a point in the past, without having to store the state at every point in history.

## Operations

Now that you know what we have to work with, let's go over the operations that `Engine` supports. Each of these operations relies on a bunch of different shared helpers, as we go from the simplest operations to the most complex, we'll gradually build up the set of helpers we use.

After describing how each operation or helper works there'll often be a code block with the actual function, it isn't necessary to understand the code and it may have complications that aren't mentioned, so feel free to skip them, they're there if you want to confirm or enhance your understanding.

### Subset helpers

`Subset` has a number of operations that produce new `Subset`s, these form the core of most work the CRDT operations do. It's better to explain them together near the start since they're used everywhere:

#### Subset::union

Takes two `Subset`s and produces a new `Subset` of the same string where each character has the sum of the counts it has in each input. When treating a `Subset` as a normal set, this is just the union.

#### Subset::transform_expand

`Subset::transform_expand` takes a `Subset` and another "transform" `Subset` and transforms the first `Subset` through the coordinate transform represented by the "transform". Now what does this mean:

`Revision`s are never modified even when the union string expands with newly inserted characters. We can deal with this by treating `Subset`s of inserted characters as coordinate transforms. The only difference to the union string is the inserted characters, so if we can modify the coordinates of a `Subset` from being based on one union string to another, we can work with edits from multiple `Revision`s together.

We can do this by "expanding" the indices in a `Subset` after each insert by the size of that insert, where the inserted characters are the "transform". Conceptually if a `Subset` represents the set of characters in a string that were inserted by an edit, then it can be used as a transform from the coordinate space before that edit to after that edit by mapping a `Subset` of the string before the insertion onto the 0-count regions of the transform `Subset`.

<!-- TODO pictures -->

#### Subset::transform_union

Like `transform_expand` except it preserves the non-zero segments of the transform instead of mapping them to 0-segments. This is the same as `transform_expand`ing and then taking the `union` with the transform. So:

```rust
a.transform_union(&b) == a.transform_expand(&b).union(&b)
```

### Engine::get_rev

This operation is used in the plugin API and is probably the simplest operation, but it still relies on a lot of sub-steps that are shared with other operations.

The idea behind how it works is that we already have all the characters we need in the `text` and `tombstones` `Rope`s we store, but some of the characters from the past revision might have been deleted, and some new characters might have been inserted that weren't in the past revision. We need to find a way to delete the newer insertions from `text` and insert the things that weren't deleted at the past point from where they are in `tombstones`.

The way we describe the current state of `text` and `tombstones` relative to the "union string" is with `deletes_from_union` (see [Engine](#engine)), so what if we could find a similar `old_deletes_from_cur_union` that represented what the old revision's text looked like relative to the current union string. This would be the same as our current `deletes_from_union` except characters inserted after the old revision would be marked deleted and newer deletes would be un-marked. The function that finds this is `Engine::deletes_from_cur_union_for_index`.

Once we have this `old_deletes_from_cur_union` and a new `deletes_from_union`, we need a way to take our current `text` and `tombstones` and get a `Rope` of what the `text` would have looked like at that old revision. We can do this by performing inserts and deletes on the `text` `Rope` based on the differences between the old and new deletions. We already have a way of describing inserts and deletes (a `Delta`), and we can create one using a helper called `Delta::synthesize`.

Then we just have to apply the `Delta` we synthesized to the current `text`, returning the resulting old text.

```rust
/// Get text of a given revision, if it can be found.
pub fn get_rev(&self, rev: RevToken) -> Option<Rope> {
    self.find_rev_token(rev).map(|rev_index| self.rev_content_for_index(rev_index))
}

/// Get text of a given revision, if it can be found.
fn rev_content_for_index(&self, rev_index: usize) -> Rope {
    let old_deletes_from_union = self.deletes_from_cur_union_for_index(rev_index);
    let delta = Delta::synthesize(&self.tombstones,
        &self.deletes_from_union, &old_deletes_from_union);
    delta.apply(&self.text)
}
```

#### Engine::deletes_from_cur_union_for_index

We can find an `old_deletes_from_cur_union` by taking our current `deletes_from_union` and walking backwards through our list of `Revision`s undoing the changes they have made since the old revision.

There's a problem with this though, the `inserts` and `deletes` subsets use indices in the coordinate space of the union string at the time the `Revision` was created, which may be smaller than our current union string.

We could keep track of the transform and account for it, but it's easier to use a helper we need anyway elsewhere that computes what the `deletes_from_union` actually would have been at that previous point in time, that is, relative to the old union string not the current one like we want. This helper is called `Engine::deletes_from_union_for_index` and it performs the work of un-deleting and un-undoing everything after the old revision.

Then we can take this `old_deletes_from_union` and `Subset::transform_union` it through the `inserts` since that old revision to get the `old_deletes_from_cur_union` we wanted. This puts it in the right coordinate space and the `union` part of `transform_union` makes sure that new inserts are considered as deleted (so not present) in `old_deletes_from_cur_union`.

```rust
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
```

#### Engine::deletes_from_union_for_index

This function uses the property that each `Revision` contains the information necessary to reverse it to work backwards from the current state of `deletes_from_union` to the past state. For every `Edit` revision it `subtract`s the `deletes` (meaning if something was deleted twice, this will only reverse one), but only if they weren't undone, and then uses `transform_shrink` to reverse the coordinate transform of the `inserts` so that the indices in the intermediate `old_deletes_from_union` refer to the previous union string. `Undo` edits store the symmetric differences of the `deletes_from_union` and the currently undone groups, so those are just reversed.

```rust
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
                    let new_undone = undone_groups.symmetric_difference(toggled_groups).cloned().collect();
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
```
