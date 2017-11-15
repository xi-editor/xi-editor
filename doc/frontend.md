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

## Methods

These are mostly described by example rather than specified in detail.
They are given in shorthand, eliding the JSON-RPC boilerplate. For
example, the actual interaction on the wire for `new_view` is:

```
to core: {"id":0,"method":"new_view","params":{}}
from core: {"id":0,"result": "view-id-1"}
```

## Top-level methods served by back-end

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

### plugin
**Note:** plugin commands are in flux, and may change.

`plugin {"method": "start", params: {"view_id": "view-id-1", plugin_name: "syntect"}}`

Dispatches the inner method to the plugin manager.

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

### edit
`edit {"method": "insert", "params": {"chars": "A"}, "view_id":
"view-id-4"}`

Dispatches the inner method to the per-tab handler, with individual
inner methods described below:


### Edit methods

#### insert

`insert {"chars":"A"}`

Inserts the `chars` string at the current cursor location.


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

### From back-end to front-end

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

### plugins

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
