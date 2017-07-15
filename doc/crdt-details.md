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

Importantly, `Subset` has a number of operations that produce new `Subset`s, these form the core of most work the CRDT operations do. Here are some of the most important ones:

#### union

Takes two `Subset`s and produces a new `Subset` of the same string where each character has the sum of the counts it has in each input. When treating a `Subset` as a normal set, this is just the union.

#### transform_expand

This allows `Subset`s to be used as the coordinate transforms mentioned earlier. Conceptually if a `Subset` represents the set of characters in a string that were inserted by an edit, then it can be used as a transform from the coordinate space before that edit to after that edit by mapping a `Subset` of the string before the insertion onto the 0-count regions of the transform `Subset`.

<!-- TODO pictures -->

#### transform_union

Like `transform_expand` except it preserves the non-zero segments of the transform instead of mapping them to 0-segments. This is the same as `transform_expand`ing and then taking the `union` with the transform. So:

```rust
a.transform_union(&b) == a.transform_expand(&b).union(&b)
```

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

A key property of `Revision`s is that they contain all the necessary information to apply them as well as reverse them. This is important both for undo and also for some CRDT operations we'll get to later. This is why `Contents::undo` stores the set of toggled groups rather than the new set of undone groups. It's also as why it stores a reversible set of changes to the deleted characters (more on those later), this could be found by replaying all of history using the new set of undo groups, but then it would be inefficient to apply and reverse (because it would be proportional to the length of history).

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

Now that you know what we have to work with, let's go over the operations that `Engine` supports. Each of these operations relies on a bunch of different shared helpers, as we go from the simplest operations to the most complex, each operation will describe the helpers it uses so that you gradually add to the set needed.
