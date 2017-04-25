# Xi view update protocol

This document describes a proposal for a new protocol for sending view updates from the core to the front-end.

## Background

Goals: keep everything async. Keep network traffic minimal. Allow front-end to retain as much information as possible (including text if only cursors are updated). Allow front-end to use small amounts of memory even when document is large.

Conceptually, the core maintains a full view of the document, which can be considered an array of lines. Each line consists of the *text* (a string), a set of *cursor* locations, and a structure representing *style* information. Many operations update this view, at which point the core sends a notification to the front-end.

The front-end maintains a *cache* of this view. Some lines will be present, others will be missing. A cache is consistent with the true state when all present lines match.

To optimize communication, the core keeps some state about the client. One bit of this state is the *scroll window;* in general, the core tries to proactively update all lines within this window (plus a certain amount of slop on top and bottom). In addition, the core maintains a set of lines in the client's cache. If a line changes, the update need only be communicated if it is in this set. This set is conservative; if a line is missing in the actual cache held by the front-end (evicted to save memory), no great harm is done updating it.

## Requests from front-end to core

```
scroll: [number, number]  // first line, last line
```

```
request: [number, number]  // first line, last line
```

## Requests from core to front-end


```
set_style
  id: number
  fg_color?: number // 32-bit RGBA value
  bg_color?: number // 32-bit RGBA value, default 0
  weight?: number // 100..900, default 400
  italic?: boolean  // default false
```

It's not hard to imagine more style properties (typeface, size, OpenType features, etc).

The guarantee on id is that it is not currently in use in any lines in the view. However, in practice, it will probably just count up. It can also be assumed to be small, so using it as an index into a dense array is reasonable.

Discussion question: should the scope of set_style be to a tab, or to the global session?

Style number 0 is reserved for selections. Discussion question: should other styles be reserved, like 1 for find results?

```
scroll_to: [number, number]  // line, column (in utf-8 code units)
```

```
update
  rev: number
  ops: Op[]

interface Op {
  op: "copy" | "skip" | "invalidate" | "update" | "ins"
  n: number  // number of lines affected
  lines?: Line[]  // only present when op is "update" or "ins"  
}
```

An update request can be seen as a function from the old client cache state to a new one. During evaluation, maintain an index (`old_ix`) into the old `lines` array, initially 0, and a new lines array, initially empty. [Note that this document specifies the semantics. The actual implementation will almost certainly represent at least initial and trailing sequences of invalid lines by their count; and the editing operations may be more efficiently done in-place than by copying from the old state to the new].

The "copy" op appends the `n` lines `[old_ix: old_ix + n]` to the new lines array, and increments `old_ix` by `n`.

The "skip" op increments `old_ix` by `n`.

The "invalidate" op appends n invalid lines to the new lines array.

The "ins" op appends new lines, specified by the "`lines`" parameter, specified in more detail below. For this op, `n` must equal `lines.length` (alternative: make n optional in this case). It does not update `old_ix`.

The "update" op updates the cursor and/or style of n existing lines. As in "ins", n must equal lines.length. It also increments `old_ix` by `n`.

In all cases, n is guaranteed positive and nonzero (as a consequence, any line present in the old state is copied at most once to the new state).

```
interface Line {
  text?: string  // present when op is "update"
  cursor?: number[]  // utf-8 code point offsets, in increasing order
  styles?: number[]  // length is a multiple of 3, see below
}
```

The interpretation of a line is different for "update" or "ins" ops. In an "ins" op, text is always present, and missing cursor or styles properties are interpreted as empty (no cursors on that line, no styles).

In an "update" op, then the text property is absent from the line, and text is copied from the previous state (or left invalid if the previous state is invalid), and the cursor and styles are updated if present. To delete cursors from a line, the core sets the cursor property to the empty list.

The styles property represents style spans, in an efficient encoding. It is conceptually an array of triples (though flattened, so triple at is `styles[i*3]`, `styles[i*3 + 1]`, `styles[i*3 + 2]`). The first element of the triple is the start index (in utf-8 code units), but encoded as a delta relative to the *end* of the last span (or relative to 0 for the first triple). It may be negative, if spans overlap. The second element is the length (in utf-8 code units). It is guaranteed nonzero and positive. The third element is a style id. The core guarantees that any style id sent in a styles property will have previously been set in a set_style request.

The number of lines in the new lines array always matches the view as maintained by the core. Another way of saying this is that adding all "`n`" values except for "skip" operations is the number of lines. [Discussion: the last line always represents a partial line, so an empty document is one empty line. But I think the initial state should be the empty array. Then, the empty array represents the state that no updates have been processed].

## Discussion questions

Should offsets be utf-8 or utf-16? The majority of front-ends use utf-16. It might be more efficient for xi to do the conversions (in Rust) than the front-end.

