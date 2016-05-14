# Notes on writing front-ends

These notes are provisional, as the protocol between front-end and
back-end (aka "core") is expected to evolve. Even so, it might be
interesting to experiment with new front-ends other than the offical
Cocoa one. This document captures the protocol as it exists (and
should be updated as it changes).

The front-end starts the core process and communicates to it through
stdin and stdout. The outer layer is based heavily on [JSON-RPC
2](http://www.jsonrpc.org/specification), communicating over stdin and
stdout, with messages encoded in UTF-8 and terminated in newlines.
However, there are two differences. Most importantly, the protocol is
peer-to-peer rather than defining strict server and client roles; both
peers can send RPC's to the other. To reflect that it is not exactly
JSON-RPC 2, the "jsonrpc" parameter is missing.

A mixture of synchronous and asynchronous RPC's is used. Most editing
commands are sent as asynchronous RPC's, with the expectation that
the core will send an (also asynchronous) `update` RPC with the
updated state.

When the front-end quits, it closes the stdin pipe, and the core
is expected to quit silently.

The protocol is currently not versioned, as there is only one
official front-end, and it is distributed along with the back-end;
both should change in lock step. That may well change if and when
there are other front-ends developed independently, in which case a
simple version negotiation at startup will support a small window of
versions.

## Methods

These are mostly described by example rather than specified in detail.
They are given in shorthand, eliding the JSON-RPC boilerplate. For
example, the actual interaction on the wire for `new_tab` is:

```
to core: {"id":0,"method":"new_tab","params":[]}
from core: {"id":0,"result":"1"}
```

## Top-level methods served by back-end

### new_tab

`new_tab []` -> `"1"`

Creates a new tab, returning the tab name as a string (currently
a number, but tab names derived from filenames might be more
debug-friendly).

### delete_tab

`delete_tab {"tab": "1"}`

Deletes a tab, which was created by `new_tab`.

`edit {"method": "insert", "params": {"chars": "A"}, tab: "0"}`

Dispatches the inner method to the per-tab handler, with individual
inner methods described below:

### Edit methods

#### key

`key {"chars":"k","flags":0,"keycode":40}`

**This method is deprecated, use `insert` and individual action
methods instead.**

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

#### insert

`insert {"chars":"A"}`

Inserts the `chars` string at the current cursor location.

#### open

`open {filename:"/Users/raph/xi-editor/rust/src/editor.rs"}`

Directs the back-end to open the named file. Note, there is currently
no mechanism for reporting errors. Also note, the protocol delegates
power to load and save arbitrary files. Thus, exposing the protocol
to any other agent than a front-end in direct control should be done
with extreme caution.

#### save

`save {filename:"/Users/raph/xi-editor/rust/src/editor.rs"}`

Similar to `open`.

#### scroll

`scroll [0,18]`

Notifies the back-end of the visible scroll region, defined as the
first and last (non-inclusive) formatted lines. The visible scroll
region is used to compute movement distance for page up and page down
commands, and also controls the size of the fragment sent in the
`update` method.

#### click

`click [42,31,0,1]`

Implements a mouse click. The array arguments are: line and column
(0-based, utf-8 code units), modifiers (again, 2 is shift), and
click count.

#### drag

`drag [42,32,0]`

Implements dragging (extending a selection). Arguments are line,
column, and flag as in `click`.

The following edit methods take no parameters, and have similar
meanings as NSView actions. This list is expected to grow.

```
delete_backward
insert_newline
move_up
move_up_and_modify_selection
move_down
move_down_and_modify_selection
move_left
move_left_and_modify_selection
move_right
move_right_and_modify_selection
scroll_page_up
page_up
page_up_and_modify_selection
scroll_page_down
page_down
page_down_and_modify_selection
```

### From back-end to front-end

#### update

```
update {"tab": "1", "update": {
 "first_line":0,
 "height":1,
 "lines":[["hello",["sel",4,5],["cursor",4]]],
 "scrollto":[0,4]
}}
```

The update method is the main way of conveying formatted text to
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

The update method is also how the back-end indicates that the
contents may have been invalidated and need to be redrawn. The
evolution of this method will probably include finer grained
invalidation (including motion of just the cursor), but will broadly
follow the existing pattern.

### RPCs from front-end to back-end

#### render_lines

`render_lines {"first_line":45,"last_line":64}` -> *lines*

A request for a "lines" array to cover the given range of formatted
lines. The response is an array with the same meaning as the
`lines` field of the `update` method.

## Other future extensions

Things the protocol will need to cover:

* Dirty state (for visual indication and dialog on unsaved changes).

* Minimal invalidation.

* General configuration options (word wrap, etc).

* Many more commands (find, replace).

* Display of autocomplete options.

* ...
