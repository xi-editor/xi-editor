# Rope science

In April and May 2016, when I was working out the design for xi editor, I wrote a series of Google-internal posts I entitled "rope science", which dug into some of the advanced computer science concepts I was hoping to employ to make xi better, as well as some more speculative explorations. I had always intended to publish these after some cleanup and polishing, but never got around to it.

For the curious and persistent, here are the original posts, with a bit of light editing and context (including cherry-picks from ensuing discussion). They will probably be helpful to understand xi internals, but don't take the place of real documentation. That said, these posts can hopefully provide input for that documentation, and may be interesting on their own.

I don't think I ever wrote a part 7. It was supposed to be about spans and interval trees, still a very interesting topic.

Enjoy!

Table of Contents:
* [01 MapReduce for Text](rope_science_01.md)
* [02 Metrics](rope_science_02.md)
* [03 Grapheme Cluster Boundaries](rope_science_03.md)
* [04 Parenthesis matching](rope_science_04.md)
* [05 Incremental Word Wrapping](rope_science_05.md)
* [06 Parallel and Asynchronous Word Wrapping](rope_science_06.md)
* [08 CRDTs for Concurrent Editing](rope_science_08.md)
* [08(a) CRDT Follow-up](rope_science_08a.md)
* [09 CRDT Approach to Async Plugins and Undo](rope_science_09.md)
* [10 Designing for a Conflict-free World](rope_science_10.md)

