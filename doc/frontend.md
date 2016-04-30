# Notes on writing front-ends

These notes are provisional, as the protocol between front-end and
back-end (aka "core") is expected to evolve. Even so, it might be
interesting to experiment with new front-ends other than the offical
Cocoa one. This document captures the protocol as it exists (and
should be updated as it changes).

The front-end starts the core process and communicates to it through
stdin and stdout. At the outer layer, messages in both directions
have a binary framing protocol, consisting of an 8-byte length in
little-endian order prepended to the message. Currently, the message
is JSON, and encoded in UTF-8.

When the front-end quits, it closes the stdin pipe, and the core
is expected to quit silently.

The protocol is currently not versioned, as there is only one
official front-end, and it is distributed along with the back-end;
both should change in lock step. That may well change if and when
there are other front-ends developed independently, in which case a
simple version negotiation at startup will support a small window of
versions.

Many messages are asynchronous, but there is an RPC mechanism
(currently supporting calls from front-end to back-end only) layered
on top of the basic message mechanism, allowing synchronous calls.

The current protocol assumes a single buffer being edited, so multiple
windows and tabs are not possible. **This will change.** It will be
extended, either by having a general buffer-selecting message that
wraps the individual messages below, or adding a buffer identifier to
each of the relevant messages. The latter is simpler but perhaps less
flexible.

## Messages

These are mostly described by example rather than specified in detail.

### From front-end to back-end

#### key

`["key",{"chars":"k","flags":0,"keycode":40}]`

Flags are the Cocoa NSEvent modifier flags shifted right 16 bits
(ie the device independent part). In particular, shift is 2.

Right now, function keys are sent as NS [function key "unicodes"](https://developer.apple.com/library/mac/documentation/Cocoa/Reference/ApplicationKit/Classes/NSEvent_Class/index.html#//apple_ref/doc/constant_group/Function_Key_Unicodes)
in the 0xF700 range, and are interpreted by the core. **This will
change, see some of the discussion in pull request #12.** In the
near future, such functions will get interpreted by the front-end
and sent as individual commands, generally following the action
descriptions in [NSResponder](https://developer.apple.com/library/mac/documentation/Cocoa/Reference/ApplicationKit/Classes/NSResponder_Class/),
such as "deleteBackward" and "pageDown".

Further, there will be full support for input methods, which among
other things will support emoji input (issue #21). I anticipate
implementing [NSTextInputClient](https://developer.apple.com/library/mac/documentation/Cocoa/Reference/NSTextInputClient_Protocol/)
in the Cocoa front-end. This is quite nontrivial and will require
lots of messages, and possibly reporting of UTF-16 code unit offsets
through the protocol. A UTF-16 counting metric will likely be added
to the rope to support this.

But sending uninterpreted keys was a good simple starting point to
get something working quickly.

#### open

`["open","/Users/raph/xi-editor/rust/src/editor.rs"]`

Directs the back-end to open the named file. Note, there is currently
no mechanism for reporting errors. Also note, the protocol delegates
power to load and save arbitrary files. Thus, exposing the protocol
to any other agent than a front-end in direct control should be done
with extreme caution.

#### save

`["save","/Users/raph/xi-editor/rust/src/editor.rs"]`

Similar to `open`.

#### scroll

`["scroll",[0,18]]`

Notifies the back-end of the visible scroll region, defined as the
first and last (non-inclusive) formatted lines. The visible scroll
region is used to compute movement distance for page up and page down
commands, and also controls the size of the fragment sent in the
`settext` message.

#### click

`["click",[42,31,0,1]]`

Implements a mouse click. The array arguments are: line and column
(0-based, utf-8 code units), modifiers (again, 2 is shift), and
click count.

#### drag

`["drag",[42,32,0]]`

Implements dragging (extending a selection). Arguments are line,
column, and flag as in `click`.

#### rpc

`["rpc",{"index":42,"request":...request body...}]`

The RPC request includes a nonce (for associating the response if
multiple RPC's are in flight at one time) and wraps a request body.

### From back-end to front-end

#### settext

```
["settext",{
 "first_line":0,
 "height":1,
 "lines":[["hello",["sel",4,5],["cursor",4]]],
 "scrollto":[0,4]
}]
```

The settext message is the main way of conveying formatted text to
display in the editor window. `first_line` is the index of the first
formatted line in the `lines` array (generally this will be the
visible region conveyed by `scroll` plus some padding). `height` is
the total number of formatted lines, and is suitable for setting the
height of the scroll region. `scrollto` is a (line, column) pair
(both 0-indexed) requesting to bring that cursor position into view.

The `lines` array has additional structure. Each line is an array,
of which the first element is the text of the line and each
additional element is an annotation. Current annotations include:

`cursor`: An offset from the beginning of the line, in UTF-8 code
units, indicating a cursor to be drawn at that location. In future,
multiple cursor annotations may be present (to support multiple
cursor editing). The offset might possibly switch to UTF-16 code
units as well, because it's probably faster to do the conversion in
Rust code than in the front-end.

`sel`: A range (expressed in UTF-8 code units) to be highlighted
indicating a selection. Note that in the case of BiDi there will
generally be at most one selection region, but it might be displayed
as multiple runs.

`fg`: A range (same as sel) and an ARGB color (4290772992 is
0xffc00000 = a nice red). Might possibly change to a symbolic
representation of the color to give the front-end more control over
theming.

The settext message is also how the back-end indicates that the
contents may have been invalidated and need to be redrawn. The
evolution of this message will probably include finer grained
invalidation (including motion of just the cursor), but will broadly
follow the existing pattern.

### RPCs from front-end to back-end

#### render_lines

`["render_lines",{"first_line":45,"last_line":64}]`

A request for a "lines" array to cover the given range of formatted
lines. The response is an array with the same meaning as the
`lines` field of the `settext` message.

## Other future extensions

Things the protocol will need to cover:

* Multiple tabs and/or windows. Discussed a bit above.

* Mouse navigation.

* Dirty state (for visual indication and dialog on unsaved changes).

* General configuration options (word wrap, etc).

* Many more commands (find, replace)

* ...
