# A CRDT approach to async plugins and undo

A primary goal of the xi project is to provide extremely fast responsiveness in all circumstances. For core editing operations, this can generally be accomplished by extremely fast implementation of the editing primitives. However, non-responsiveness due to plugins is a very significant problem, and just making faster plugins is not an appealing solution; we want plugins to do sophisticated lexical and syntactical analysis so their operations can be accurate.

Other editor projects recognize delays due to plugins and propose to run the plugins asynchronously. [Neovim](https://github.com/neovim/neovim/wiki/Plugin-UI-architecture) is probably the most advanced in this regard. However, simply running the editing operations asynchronously carries its own risk: the user and the plugin (or perhaps multiple plugins) are _racing,_ and the final editing result can depend on the order of execution.

This problem is fundamentally similar to collaborative editing, and there is a literature spanning decades. Some of the approaches, such as [operational transformation](https://en.wikipedia.org/wiki/Operational_transformation), are fiendishly difficult to implement correctly, and seem like overkill for a primarily single-user text editor. However, in recent years, the Conflict-free Replicated Data Type (CRDT) approach has emerged, and I believe it is well suited for xi. In particular, I believe a centralized CRDT implementation in the core can be implemented with only a bit more complexity than required for undo and running plugins asynchronously, and yet provide strong eventual consistency guarantees characteristic of CRDTs.

In addition, should xi ever be extended to collaborative editing, having the model be based on the CRDT abstraction will make implementation much easier, leaving only the small matter of efficient distributed CRDTs themselves.

This document is structured into two main sections. The first presents the model at a conceptual level, and effectively functions as a spec for how out-of-order edits should be merged, as well as the desired semantics for undo. The second details a simple and efficient centralized implementation, which faithfully implements the model but bypasses the complexity of a true distributed environment.

## CRDT model

The document consists of a collection of characters, each of which has a stable unique id. In addition, there are _ordering edges_ between pairs of characters. For example, if the document is "AB" and the user inserts an "X" between these two characters, the ordering edges A < X and X < B are generated. Two special id's represent the beginning and end of the document.

If only a single user were editing the document, then these edges would form a total order. However, in the face of concurrent edits, they are in general only a partial order. For example, if another user inserts "Y" at the same location, then the additional edges A < Y and Y < B are generated, but there is no order between X and Y. In this case, we define tie-breaking rules based on an integer id of each user. For example, assume the first user has the lower id. Then, with the edits applied in either order, the result will consistently be "AXYB". This approach follows WOOT.

### Deletion; tombstones; intervals

In a CRDT framework, the user action of deleting a character doesn't remove it from the graph, it simply marks it as deleted. Thus, the merge operation forms a monotonic semi-lattice; the union of new characters and ordering edges, and the "or" of delete state for the character.

A deleted character is known as a "tombstone", and these have also been used to patch up operational transformations to fix the failure to achieve eventual consistency (this is known in the literature as the TP2 puzzle and is well explained in the tombstone paper referenced below). Retaining tombstones doubly makes sense in an editor because there is a chance the deletion will be undone, so retaining it guarantees that its order relative to its context is preserved.

In xi, rather than retaining a deleted state for each individual character, we will represent deletions as _intervals._ This still fits well into the CRDT model; the delete state is a set of intervals, and the merge operation is simple set union (also obviously a monotonic semilattice).

An interval is defined as a start point and an end point. Each point is either _before_ or _after_ a given character. In the example above, if the document is "AB", then "after A" and "before B" initially refer to the same cursor location, but would become different after insertion between the two.

A consequence of the interval approach is that some inserts can get lost. For example, if the document is "AB", user 1 deletes "AB", and user 2 concurrently inserts "X" between A and B, then the resulting document is empty. In the CRDT view, the characters are A, B, and X, the ordering edges are A < B, A < X, and X < B, and the deleted interval set is {(before A, after B)}. This is a different state than would be the case in WOOT, which would mark A and B as deleted but retain X, but I think quite reasonable semantically.

Also note that the interval approach is strictly more general; if the per-character granularity of deletion is desired, it can be simulated by creating a (before C, after C) interval for each character to be deleted.

### Undo

Defining the semantics of undo is tricky, especially in the face of concurrent edits, and a lot of ink has been spilled on the subject. Here we present a simple but general model again based on CRDT.

Each editing operation is assigned an "undo group." Several edits may be in the same group. For example, if the user types `"`, then a smart-quote plugin may revise that to '“'. If the smart-quote revision is assigned the same undo group (because it is a consequence of the same user action), then a single undo would zorch both edits. Note, incidentally, that TextEdit on MacOS X does not exhibit this behavior. The first undo leaves the buffer with '"' (and selected), and it requires a second undo to return to the intial contents.

Each edit has _two_ sets intervals, one for deletions (as above), but also for insertions. The set of characters covered by the insertion intervals should be exactly the set of new characters introduced by the edit. In normal editing, the insertion intervals are ignored. However, when an undo group goes into an undone state, the insertion and deletion intervals are swapped. Note that if a character is both inserted and deleted by edits in the same undo group (as is the case for the poor dumb quote above), its state does not change.

Note that the mechanism allows for selective undo of any undo group. We probably won't expose this generality through the UI, but rather provide the familiar ctrl-z, shift-ctrl-z keybindings. However, undoing complex edits interleaved with others (because of slow plugins) will call upon this generality.

I'm not overly concerned with concurrent modification to undo state, as it will be managed by the central core, but it's still straightforward to handle as a CRDT. Each undo group gets a distributed counter, and the group is considered to be undone when the counter is odd-valued.

### Recap

It's worth recapping the end-to-end flow, because it's not obvious. Reconstructing the visible buffer contents from the CRDT state will be described as a batch operation. Obviously we want to compute that incrementally, but implementation details add complexity and are very different depending on whether it is centralized or distributed.

The CRDT state is:

* A set of characters, each given by stable unique id (and for which two id's can be compared for tie-breaking purposes).

* A set of ordering edges, each of which is a pair of ids.

* A set of edits, each of which consists of:

  - An undo group id.

  - A set of delete intervals.

  - A set of insert intervals.

These are all basically sets, so the CRDT merge operation is just set union of each of these components.

To reconstruct the buffer:

* Topologically sort the characters by ordering edges and tiebreaks, yielding a sequence.

* Decide for each edit group whether it is in an undo state, either centrally or by taking the parity of its associated counter.

* For each edit, if its edit group is in an undone state, add its insert intervals, otherwise add its delete intervals.

* Take the union of all intervals from the previous step, and delete those from the sequence.

Et voila!

## Efficient implementation

Details to come later, but I'll try to sketch out rough ideas.

In xi, we're not setting out to do a fully distributed peer-to-peer CRDT implementation (though such a thing would be a very interesting future extension). Rather, we want to exploit the assumption that the core is fast and reliable, and have it evaluate the CRDT efficiently and incrementally, but in serial.

Instead of storing the CRDT components explicitly, we store a sequence of _snapshots._ Following the CRDT model, each snapshot contains the "union string" (the result of the topological sort, with deleted sections still present). However, we don't store explicit unique ids for each character, or explicit ordering edges. Rather, the total order (with tie-breaking applied) is preserved in each snapshot.

Because we don't have explicit unique id's, we simulate them by doing _coordinate transforms._ This is fairly straightforward because the only difference between the union strings in two different snapshots is intervals of inserted characters. In fact, we can probably re-use the "insert intervals" retained for undo purposes as the concrete representation of the needed coordinate transforms.

The coordinate transform approach has an operational transform flavor, but it is important to note, the goal is still to implement exactly the same CRDT computation as specified in the first section.

We don't retain all snapshots going back to the beginning of history, nor do we retain all tombstones indefinitely. Rather, we _garbage collect_ a snapshot as soon as it is no longer needed (because it expired out of the undo buffer and we are sure its undo state will not toggle, and because it is not used as the base of any in-flight plugin operation). Garbage collection also removes deleted intervals when there is no possible future reference to them.

In the general case, undo requires going back to an earlier snapshot (the latest one that doesn't contain any edits from the undo group being toggled), and replaying the history from there. Because of this and the need for garbage collection, the undo history will be bounded rather than infinite (20 steps seems reasonable).

## References

[An Undo Framework for P2P Collaborative Editing](https://hal.archives-ouvertes.fr/inria-00432373/document)

[Tombstone Transformation Functions for Ensuring Consistency in Collaborative Editing Systems](http://www.loria.fr/~urso/uploads/Main/oster06collcom.pdf)

[Real time group editors without Operational transformation](https://hal.inria.fr/inria-00071240/document) (WOOT)

[A comprehensive study of Convergent and Commutative Replicated Data Types](http://hal.upmc.fr/inria-00555588/document)
