---
layout: page
title: CRDT - Using the Ledger for CRDTs
site_nav_category_order: 205
is_site_nav_category2: true
site_nav_category: docs
---

This document contains notes about implementing a CRDT on top of the Ledger on Fuchsia, based on existing and planned work for Xi's text CRDT. It should be helpful for anyone developing a CRDT that works in the form of a Ledger custom conflict resolver that merges two histories.

## Basic Setup

When you open a document that is synced with the ledger you want to do a few things:

1. Set a conflict resolver factory that uses your CRDT merge if you haven't already done so.
1. Find the Ledger page ID that corresponds with the document, you could do this by hashing the document ID, storing references to IDs in the root page, or storing a randomly generated page ID in the story Link.
1. Get that page and call `GetSnapshot` to get a snapshot, also register a `PageWatcher` in the same call.
1. Load the initial state of your document from the page snapshot into your local in-memory state.
1. Start listening on the `PageWatcher` for updates. It's important to start listening after you load or you might encounter race conditions.
1. Whenever your `PageWatcher` recieves an update you want to use your CRDT `merge` to merge the updated state into your local state. You don't want to just load it because you may have pending edits in your local state that haven't made it into the ledger yet, see the next section. Make sure you respond to the FIDL message only after you are done incorporating the changes.

## Handling Edits

When an edit is made by the user that arrives through some event, you want to store that change in the Ledger as soon as possible so that other users get it, but doing so with minimal latency without violating CRDT properties is non-trivial.

Below is the procedure we've came up with:

1. Apply the edit to your local state. Feel free to update the view right away.
1. Call `StartTransaction` on the `Page` and wait for the result.
1. The `StartTransaction` call will only return once all pending `PageWatcher` updates have returned. So in the mean time all pending updates to the state will be processed, then the visible state of the `Page` will be frozen so that your local state reflects the current visible `Page` state.
1. Once the `StartTransaction` call returns, do one of two things depending on how you represent your CRDT in Ledger:
    - If your state as stored as a single key or structure that gets updated as a whole, `Put` your local state into the Ledger.
    - If your state is stored as a multi-key history which must only be appended to, load the current state of the `Page` and `merge` your local state into the loaded state, then `Put` the new appended edits to the Ledger. If you just put your local history, it might have a different ordering even if it semantically contains the same edits, violating the append-only constraint. The append-only nature also prevents thrashing where two peers alternative overwriting the whole history with their own ordering, which would bloat the Ledger history and eliminate the ability to get minimal diffs through changed keys.
1. Call `Commit` on the `Page`

There's alternative ways you can do this, but they tend to require waiting for round-trips to the Ledger in order for edits to reach the screen. They also tend to rely more heavily on the conflict resolver, which is probably more expensive than just using your `merge` operation in-process.

## Representation

It's normally easiest to start by storing your entire CRDT serialized into one Ledger key, but this doesn't scale to large instances.

Here's some ideas for representations we've come up with:

#### Lists

For large lists where you want to be able to insert and delete ranges anywhere in the list incrementally, for example the list of characters in the current text of a document, there's a couple options:

You can use a linked list of chunks with a maximum and minimum length. Each chunk uses a key which is just some 64 bit number, and a value which is the chunk content. When you insert you modify the corresponding chunk and if it grows bigger than the maximum length you split it into multiple chunks, modifying the sibling pointers of adjacent chunks. If you delete and a chunk gets too small you merge it with adjacent chunk(s). This allows the size of the Ledger commit to be proportional to the size of the change, as well as allowing you to use the changed keys in page watchers and conflicts to get a conservative diff that allows you to avoid loading the whole document. If just the linear search to find the correct chunk to insert in is too slow, you can make it a skip list or a balanced tree.

Alternatively, instead of making it a linked list you can use the keys to determine the ordering. When you insert a chunk between two other chunks you use a longer key that sorts between the keys of the two chunks you are between. Deleting a chunk just requires deleting that pair. This is similar to the previous method except it requires modifying less keys per edit, but has unbounded key growth.

#### Histories

For append-only histories, it's easy to store each revision as a separate pair with an incrementing counter for the key. The key ordering determines the order of history. Change size is just the size of the revision, and the changed keys tells you where the new edits are.

For longer histories, especially ones that are compressible, we want to batch edits together into chunks so that we waste less space on keys and also can compress edits within a chunk.

We can do this by just chunking every N (say 100) revisions together, with the last chunk potentially having less than N. Then appending just modifies the last chunk or adds a new chunk. Ledger's rolling hash for values will compress shared prefixes so that appending to a chunk doesn't bloat storage. However, it still makes finding new edits via differing keys less fine-grained and requires serializing and sending the entire chunk.

Alternatively, whenever a new edit is appended it can be added as its own key-value pair. Every once in a while a rollup process takes the last N individual edits and batches them into a chunk. This gives smaller transfers and more minimal key diffs, but is potentially more complicated.

## Other Notes

- If your CRDT is based on appending to a history, make sure that the left side of the merge is the one appended to in your conflict resolver, since results are submitted as differences from the left side.
- Ensure the Ledger representation of the result of a CRDT merge is deterministic, otherwise convergence will take much longer under concurrent editing. The easiest way to run afoul of this is using randomly-generated IDs.
- The result of a conflict you are asked to resolve is not guaranteed to become the current state yet, don't set it as your local state, wait for the `PageWatcher` update from Ledger.
- Commits can fail when out of disk space or memory. You should listen to the result of the `Commit` call to display some error state to user so they stop typing anything they care about. You also need to listen to wait for last commit before shutting down, or it may or may not be completed.
