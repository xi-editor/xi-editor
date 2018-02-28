---
layout: page
title: Front-end protocol
site_nav_category_order: 201
is_site_nav_category2: true
site_nav_category: docs
---

Please note, protocol has been [updated](#xi-view-update-protocol).

---

# Notes on writing front-ends

These notes are provisional, as the protocol between front-end and
back-end (aka "core") is expected to evolve. Even so, it might be
interesting to experiment with new front-ends other than the official
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

### Additional Resources

This document is not always perfectly up to date. For a comprehensive list of
supported commands, the canonical resource is the source specifically [rust/core-lib/src/rpc.rs](https://github.com/google/xi-editor/blob/master/rust/core-lib/src/rpc.rs).

- The update protocol is explained in more detail in [doc/update.md](https://github.com/google/xi-editor/blob/master/doc/update.md).
- The config system is explained in more detail in [doc/config.md](https://github.com/google/xi-editor/blob/master/doc/config.md).


## Table of Contents

- [API Methods](#methods)
    - [Backend](#from-front-end-to-back-end)
        - [Edit Commands](#edit-namespace)
        - [Plugin Commands](#plugin-namespace)
    - [Frontend](#from-back-end-to-front-end)

----


## Methods

These are mostly described by example rather than specified in detail.
They are given in shorthand, eliding the JSON-RPC boilerplate. For
example, the actual interaction on the wire for `new_view` is:

```
to core: {"id":0,"method":"new_view","params":{}}
from core: {"id":0,"result": "view-id-1"}
```

## From front-end to back-end

### new_view

`new_view { "file_path": "path.md"? }` -> `"view-id-1"`

Creates a new view, returning the view identifier as a string.
`file_path` is optional; if specified, the file is loaded into a new
buffer; if not a new empty buffer is created. Currently, only a
single view into a given file can be open at a time.

**Note**, there is currently no mechanism for reporting errors. Also
note, the protocol delegates power to load and save arbitrary files.
Thus, exposing the protocol to any other agent than a front-end in
direct control should be done with extreme caution.

### close_view

`close_view {"view_id": "view-id-1"}`

Closes the view associated with this `view_id`.

### save

`save {"view_id": "view-id-4", "file_path": "save.txt"}`

Saves the buffer associated with `view_id` to `file_path`. See the
note for `new_view`. Errors are not currently reported.

### set_theme

`set_theme {"theme_name": "InspiredGitHub"}`

Requests that core change the theme. If the change succeeds the client
will receive a `theme_changed` notification.

### modify_user_config

`modify_user_config { "domain": Domain, "changes": Object }`

Modifies the user's config settings for the given domain. `Domain` should be
either the string `"general"` or an object of the form `{"syntax": "rust"}`, or
`{"user_override": "view-id-1"}`, where `"rust"` is any valid syntax identifier,
and `"view-id-1"` is the identifier of any open view.

### get_config

`get_config {"view_id": "view-id-1"} -> Object`

Returns the config table for the view associated with this `view_id`.

### edit namespace
------
`edit {"method": "insert", "params": {"chars": "A"}, "view_id":
"view-id-4"}`

Dispatches the inner method to the per-tab handler, with individual
inner methods described below:


### Edit methods

#### insert

`insert {"chars":"A"}`

Inserts the `chars` string at the current cursor location.

#### cancel_operation

`cancel_operation`

Currently, this collapses selections and multiple cursors, and dehighlights
searches.

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

#### gesture

`gesture {"line": 42, "col": 31, "ty": "toggle_sel"}`

Note: both `click` and `drag` functionality will be migrated to
additional `ty` options for `gesture`. For now, "toggle_sel" is the
only supported option, and has the semantics of toggling one cursor
in the selection (the usual mapping of Command-click in macOS
front-ends).

The following edit methods take no parameters, and have similar
meanings as NSView actions. The pure movement and selection
modification methods will be migrated to a more general method
that takes a "movement" enum as a parameter.

```
delete_backward
delete_forward
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

### Plugin namespace
**Note:** plugin commands are in flux, and may change.

**Example**: The following RPC dispatches the inner method to the plugin manager.

`plugin {"method": "start", params: {"view_id": "view-id-1", plugin_name: "syntect"}}`

----

### Plugin methods

#### start

`start {"view_id": "view-id-1", "plugin_name": "syntect"}`

Starts the named plugin for the given view.


#### stop

`stop {"view_id": "view-id-1", "plugin_name": "syntect"}`

Stops the named plugin for the given view.

#### plugin_rpc

```
plugin_rpc {"view_id": "view-id-1", "receiver": "syntect",
            "notification": {
                "method": "custom_method",
                "params": {"foo": "bar"},
            }}
 ```

Sends a custom rpc command to the named receiver. This may be a notification
or a request.


## From back-end to front-end

#### update
**Note**: This document is not entirely up to date: some changes to
the protocol are described in [this document](https://github.com/google/xi-editor/blob/master/doc/update.md).

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

#### theme_changed

`theme_changed {"name": "InspiredGitHub", "theme": Theme}`

Notifies the client that the theme has been changed. The client should
use the new theme to set colors as appropriate. The `Theme` object is
directly serialized from a [`syntect::highlighting::ThemeSettings`](https://github.com/trishume/syntect/blob/master/src/highlighting/theme.rs#L27)
instance.

#### config_changed

`config_changed {"view_id": "view-id-1", "changes": {} }`

Notifies the client that the config settings for a view have changed.
This is called once when a new view is created, with `changes` containing
all config settings; afterwards `changes` only contains the key/value
pairs that have new values.

#### available_plugins

`available_plugins {"view_id": "view-id-1", "plugins": [{"name": "syntect",
"running": true]}`

Notifies the client of the plugins available to the given view.

#### plugin_started

`plugin_started {"view_id": "view-id-1", "plugin": "syntect"}`

Notifies the client that the named plugin is running.

#### plugin_stopped

`plugin_stopped {"view_id": "view-id-1", "plugin": "syntect", "code" 101}`

Notifies the client that the named plugin has stopped. The `code` field is an
integer exit code; currently 0 indicates a user-initiated exit and 1 indicates
an abnormal exit, i.e. a plugin crash.

#### update_cmds

`update_cmds {"view_id": "view-id-1", "plugin", "syntect", "cmds": [Command]}`

Notifies the client of a change in the available commands for a given plugin.
The `cmds` field is a list of all commands currently available to this plugin.
Clients should store commands on a per-plugin basis; when the `cmds` argument is
an empty list it means that this plugin is providing no commands; any previously
available commands should be disabled.

The format for describing a `Command` is in flux. The best place to look for
a working example is in the tests in core-lib/src/plugins/manifest.rs. As of
this writing, the following is valid json for a `Command` object:

```json
    {
        "title": "Test Command",
        "description": "Passes the current test",
        "rpc_cmd": {
            "rpc_type": "notification",
            "method": "test.cmd",
            "params": {
                "view": "",
                "non_arg": "plugin supplied value",
                "arg_one": "",
                "arg_two": ""
            }
        },
        "args": [
            {
                "title": "First argument",
                "description": "Indicates something",
                "key": "arg_one",
                "arg_type": "Bool"
            },
            {
                "title": "Favourite Number",
                "description": "A number used in a test.",
                "key": "arg_two",
                "arg_type": "Choice",
                "options": [
                    {"title": "Five", "value": 5},
                    {"title": "Ten", "value": 10}
                ]
            }
        ]
    }
```

## Other future extensions

Things the protocol will need to cover:

* Dirty state (for visual indication and dialog on unsaved changes).

* Minimal invalidation.

* General configuration options (word wrap, etc).

* Many more commands (find, replace).

* Display of autocomplete options.

* ...

---

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
  underline?: boolean // default false
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
  rev?: number
  ops: Op[]
  view-id: string
  pristine: bool

interface Op {
  op: "copy" | "skip" | "invalidate" | "update" | "ins"
  n: number  // number of lines affected
  lines?: Line[]  // only present when op is "update" or "ins"
}
```

The `pristine` flag indicates whether or not, after this update, this document has unsaved changes.

The `rev` field is not present in current builds, but will be at some point in the future.

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
