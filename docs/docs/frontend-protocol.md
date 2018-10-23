---
layout: page
title: The Frontend Protocol
site_nav_category_order: 202
is_site_nav_category2: true
site_nav_category: docs
---

## Table Of Contents

- [API Methods](#methods)
    - [Backend](#from-front-end-to-back-end)
        - [Edit Commands](#edit-namespace)
        - [Plugin Commands](#plugin-namespace)

    - [Frontend](#from-back-end-to-front-end)
        - [Status Bar Commands](#status-bar-commands)

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

### client_started

`client_started {"config_dir" "some/path"?, "client_extras_dir":
"some/other/path"?}`

Sent by the client immediately after establishing the core connection. This is
used to perform initial setup. The two arguments are optional; the `config_dir`
points to a directory where the user's config files and plugins live, and the
`client_extras_dir` points to a directory where the frontend can package
additional resources, such as bundled plugins.

### new_view

`new_view { "file_path": "path.md"? }` -> `"view-id-1"`

Creates a new view, returning the view identifier as a string.
`file_path` is optional; if specified, the file is loaded into a new
buffer; if not a new empty buffer is created. Currently, only a
single view into a given file can be open at a time.

**Note:**, there is currently no mechanism for reporting errors. Also
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

Asks core to change the theme. If the change succeeds the client
will receive a `theme_changed` notification.

### set_language
`set_language {"view-id":"view-id-1", "language_id":"Rust"}`

Asks core to change the language of the buffer associated with the `view_id`.
If the change succeeds the client will receive a `language_changed` notification.

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

Inserts the `chars` string at the current cursor locations.

#### paste

`paste {"chars": "password"}`

Inserts the `chars` string at the current cursor locations. If there are
multiple cursors and `chars` has the same number of lines as there are
cursors, one line will be inserted at each cursor, in order; otherwise the full
string will be inserted at each cursor.

#### copy

`copy -> String|Null`

Copies the active selection, returning their contents or `Null` if the selection was empty.

#### cut

`cut -> String|Null`

Cut the active selection, returning their contents or `Null` if the selection was empty.

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

#### resize

`resize {width: 420, height: 400}`

Notifies the backend that the size of the view has changed. This is
used for word wrapping, if enabled. Width and height are specified
in px units / points, not display pixels.

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

**Note:** both `click` and `drag` functionality will be migrated to
additional `ty` options for `gesture`.

Currently, the following gestures are supported:

```
point_select # moves the cursor to a point
toggle_sel # adds or removes a selection at a point
range_select # modifies the selection to include a point (shift+click)
line_select # sets the selection to a given line
word_select # sets the selection to a given word
multi_line_select # adds a line to the selection
multi_word_select # adds a word to the selection
```

#### goto_line

`goto_line {"line": 1}`

Sets the cursor to the beginning of the provided `line` and scrolls to
this position.

#### Other movement and deletion commands

The following edit methods take no parameters, and have similar
meanings as NSView actions. The pure movement and selection
modification methods will be migrated to a more general method
that takes a "movement" enum as a parameter.

```
delete_backward
delete_forward
insert_newline
duplicate_line
move_up
move_up_and_modify_selection
move_down
move_down_and_modify_selection
move_left
move_left_and_modify_selection
move_right
move_right_and_modify_selection
scroll_page_up
page_up_and_modify_selection
scroll_page_down
page_down_and_modify_selection
yank
transpose
select_all
add_selection_above
add_selection_below
```

#### Transformations

The following methods act by modifying the current selection.

```
uppercase
lowercase
capitalize
indent
outdent
```

#### Number Transformations

The following methods work with a caret or multiple selections. If the beginning of a selection (or the caret) is within a positive or negative number, the number will be transformed accordingly:

```
increase_number
decrease_number
```

#### Recording

These methods allow manipulation and playback of event recordings.

- If there is no currently active recording, start recording events under the provided name.
- If there is no provided name, the current recording is saved.
- If the name provided matches the current recording name, the current recording is saved.
- If the name provided does not match the current recording name, the events for the current recording are dismissed.
```
toggle_recording {
    "recording_name"?: string
}
```

Execute a set of recorded events and modify the document state:
```
play_recording {
    "recording_name": string
}
```

Completely remove a specific recording:
```
clear_recording {
    "recording_name": string
}
```

### Language Support Oriented features (in Edit Namespace)

#### Hover
Get Hover for a position in file. The request for *hover* is made as a notification. The client is forwarded result back via a `show_hover` rpc

If position is skipped in the request, current cursor position will be used in core.

```
request_hover {
    "request_id": number,
    "position"?: Position
}
```

```ts
interface Position {
    line: number,
    column: number,
}
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


### Find and replace methods

#### find

`find {"chars": "a", "case_sensitive": false, "regex": false, "whole_words": true}`
Parameters `regex` and `whole_words` are optional and by default `false`.

Sets the current search query and options.

#### multi_find

This find command supports multiple search queries.

`multi_find [{"id": 1, "chars": "a", "case_sensitive": false, "regex": false, "whole_words": true}]`
Parameters `regex` and `whole_words` are optional and by default `false`. `id` is an optional parameter
used to uniquely identify a search query. If left empty, the query is considered as a new query and
the backend will generate a new ID.

Sets the current search queries and options.

#### find_next and find_previous

`find_next {"wrap_around": true, "allow_same": false, "modify_selection": "set"}`
`find_previous {"wrap_around": true, "allow_same": false, "modify_selection": "set"}`
All parameters are optional. Boolean parameters are by default `false` and `modify_selection`
is `set` by default. If `allow_same` is set to `true` the current selection is considered a
valid next occurrence. Supported options for `modify_selection` are:
* `none`: the selection is not modified
* `set`: the next/previous match will be set as the new selection
* `add`: the next/previous match will be added to the current selection
* `add_remove_current`: the previously added selection will be removed and the next/previous
match will be added to the current selection

Selects the next/previous occurrence matching the search query.

#### find_all

`find_all { }`

Selects all occurrences matching the search query.

#### highlight_find

`highlight_find {"visible": true}`

Shows/hides active search highlights.

#### selection_for_find

`selection_for_find {"case_sensitive": false}`
The parameter `case_sensitive` is optional and `false` if not set.

Sets the current selection as the search query.

#### replace

`replace {"chars": "a", "preserve_case": false}`
The parameter `preserve_case` is currently not implemented and ignored.

Sets the replacement string.

#### selection_for_replace

`selection_for_replace {"case_sensitive": false}`
The parameter `case_sensitive` is optional and `false` if not set.

Sets the current selection as the replacement string.

#### replace_next

`replace_next { }`

Replaces the next matching occurrence with the replacement string.

#### replace_all

`replace_all { }`

Replaces all matching occurrences with the replacement string.

#### selection_into_lines

`selection_into_lines { }`

Splits all current selections into lines.

## From back-end to front-end

### View update protocol

The following three methods are used to update the view's contents. The design
of the view update protocol, has a few particular goals in mind:

- Keep everything async.
- Keep network traffic minimal.
- Allow the front-end to retain as much information as possible (including text
  if only cursors are updated).
- Allow the front-end to use small amounts of memory even when document is
  large.

Conceptually, the core maintains a full view of the document, which can be
considered an array of lines. Each line consists of the *text* (a string), a set
of *cursor* locations, and a structure representing *style* information. Many
operations update this view, at which point the core sends an `update`
notification to the front-end.

The front-end maintains a *cache* of this view. Some lines will be present,
others will be missing. A cache is consistent with the true state when all
present lines match.

To optimize communication, the core keeps some state about the client. One bit
of this state is the *scroll window;* in general, the core tries to proactively
update all lines within this window (plus a certain amount of slop on top and
bottom). In addition, the core maintains a set of lines in the client's cache.
If a line changes, the update need only be communicated if it is in this set.
This set is conservative; if a line is missing in the actual cache held by the
front-end (evicted to save memory), no great harm is done updating it. The
frontend reports this scroll window to the core by using the `scroll` method of
the `edit` notification.

#### def_style

```
def_style
  id: number
  fg_color?: number // 32-bit ARGB (word-order) value
  bg_color?: number // 32-bit ARGB (word-order) value, default 0
  weight?: number // 100..900, default 400
  italic?: boolean  // default false
  underline?: boolean // default false
```

(It's not hard to imagine more style properties such as typeface, size, OpenType
features, etc).

The guarantee on `id` is that it is not currently in use in any lines in the
view. However, in practice, it will probably just count up. It can also be
assumed to be small, so using it as an index into a dense array is reasonable.

There are two reserved style IDs, so new style IDs will begin at 2. Style ID 0
is reserved for selections and ID 1 is reserved for find results.

#### scroll_to

```
scroll_to: [number, number]  // line, column (in utf-8 code units)
```

This notification indicates that the frontend should scroll its cursor to the
given line and column.

#### update

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

The `pristine` flag indicates whether or not, after this update, this document
has unsaved changes.

The `rev` field is not present in current builds, but will be at some point in
the future.

An update request can be seen as a function from the old client cache state to a
new one. During evaluation, maintain an index (`old_ix`) into the old `lines`
array, initially 0, and a new lines array, initially empty. [Note that this
document specifies the semantics. The actual implementation will almost
certainly represent at least initial and trailing sequences of invalid lines by
their count; and the editing operations may be more efficiently done in-place
than by copying from the old state to the new].

The "copy" op appends the `n` lines `[old_ix: old_ix + n]` to the new lines
array, and increments `old_ix` by `n`.

The "skip" op increments `old_ix` by `n`.

The "invalidate" op appends n invalid lines to the new lines array.

The "ins" op appends new lines, specified by the "`lines`" parameter, specified
in more detail below. For this op, `n` must equal `lines.length` (alternative:
make n optional in this case). It does not update `old_ix`.

The "update" op updates the cursor and/or style of n existing lines. As in
"ins", n must equal lines.length. It also increments `old_ix` by `n`.

**Note:** The "update" op is not currently used by core.

In all cases, n is guaranteed positive and nonzero (as a consequence, any line
present in the old state is copied at most once to the new state).

```
interface Line {
  text?: string  // present when op is "update"
  cursor?: number[]  // utf-8 code point offsets, in increasing order
  styles?: number[]  // length is a multiple of 3, see below
}
```

The interpretation of a line is different for "update" or "ins" ops. In an "ins"
op, text is always present, and missing cursor or styles properties are
interpreted as empty (no cursors on that line, no styles).

In an "update" op, then the text property is absent from the line, and text is
copied from the previous state (or left invalid if the previous state is
invalid), and the cursor and styles are updated if present. To delete cursors
from a line, the core sets the cursor property to the empty list.

The styles property represents style spans, in an efficient encoding. It is
conceptually an array of triples (though flattened, so triple at is
`styles[i*3]`, `styles[i*3 + 1]`, `styles[i*3 + 2]`). The first element of the
triple is the start index (in utf-8 code units), but encoded as a delta relative
to the *end* of the last span (or relative to 0 for the first triple). It may be
negative, if spans overlap. The second element is the length (in utf-8 code
units). It is guaranteed nonzero and positive. The third element is a style id.
The core guarantees that any style id sent in a styles property will have
previously been set in a set_style request.

The number of lines in the new lines array always matches the view as maintained
by the core. Another way of saying this is that adding all "`n`" values except
for "skip" operations is the number of lines. [Discussion: the last line always
represents a partial line, so an empty document is one empty line. But I think
the initial state should be the empty array. Then, the empty array represents
the state that no updates have been processed].

```
interface Line {
  text?: string  // present when op is "update"
  cursor?: number[]  // utf-8 code point offsets, in increasing order
  styles?: number[]  // length is a multiple of 3, see below
}
```

---

#### theme_changed

`theme_changed {"name": "InspiredGitHub", "theme": Theme}`

Notifies the client that the theme has been changed. The client should
use the new theme to set colors as appropriate. The `Theme` object is
directly serialized from a [`syntect::highlighting::ThemeSettings`](https://github.com/trishume/syntect/blob/master/src/highlighting/theme.rs#L27)
instance.


#### available_themes

`available_themes {"themes": ["InspiredGitHub"]}`

Notifies the client of the available themes.

#### language_changed

`language_changed {"view_id": "view-id-1", "language_id": "Rust"}`

Notifies the client that the language used for syntax highlighting has been changed.

#### available_languages

`available_languages {"languages": ["Rust"]}`

Notifies the client of the available languages.

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

### Language Support Specific Commands

#### Show Hover

`show_hover { request_id: number, result: string }`

### Status Bar Commands

#### add_status_item

`add_status_item { "source": "status_example", "key": "my_key", "value": "hello", "alignment": "left" }`

Adds a status item, which will be displayed on the frontend's status bar. Status items have a reference to whichever plugin added them. The alignment key dictates whether this item appears on the left side or the right side of the bar. This alignment can only be set when the item is added.

#### update_status_item

`update_status_item { "key": "my_key", "value": "hello"}`

Update a status item with the specified key with the new value.

#### remove_status_item

`remove_status_item { "key": "my_key" }`

Removes a status item from the front end.

### Find and replace commands

#### find_status

Find supports multiple search queries.

`find_status {"view_id": "view-id-1", "queries": [{"id": 1, "chars": "a", "case_sensitive": false, "is_regex": false, "whole_words": true, "matches": 6}]}`

Notifies the client about the current search queries and search options.

#### replace_status

`replace_status {"view_id": "view-id-1", "status": {"chars": "a", "preserve_case": false}}`

Notifies the client about the current replacement string and replace options.

## Other future extensions

Things the protocol will need to cover:

* Dirty state (for visual indication and dialog on unsaved changes).

* Minimal invalidation.

* General configuration options (word wrap, etc).

* Display of autocomplete options.

* ...
