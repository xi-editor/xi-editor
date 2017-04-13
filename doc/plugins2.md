# Next steps for plugins

This document builds on the [existing plugins rfc](https://github.com/google/xi-editor/pull/43), with a focus on fleshing out a preliminary API + implementation that allows people to more easily & practically experiment with writing plugins for Xi.

## Goals:

This document aims to present an alpha/preliminary/experimental implementation that supports the global, editor-local, and one-shot/invocation plugin scopes described in the previous RFC. 

## Plugin Capabilities + UI support

For this preliminary implementation, I would like to provide basic support for a small subset of plugin capabilities:

- _commands_ are actions a plugin can define which the user can specifically invoke, for instance through a command palette. 
- _hooks_ are editor events that a plugin can be notified of, such as saving or opening a file or inserting text.
- _exports_, similar to atom's [services](http://flight-manual.atom.io/behind-atom/sections/interacting-with-other-packages-via-services/); functionality that a plugin can provide to other plugins, as a list of named RPC methods. 

And I would like to add a few simple UI elements to the editor, to make experimenting a bit easier:
- a simple _command palette_, which will be automatically populated with commands provided by plugins,
- _status fields_, elements a plugin can add to the bottom status bar of a window, and send ongoing updates to. A good simple UI paradigm, suitable for testing plugin->client interactions; 

and then, a little later:
- the ability to annotate the gutter (warnings/errors/lints)
- an autocompletion UI/API (probably as similar as possible to the language server protocol)
- inline documentation popovers


_Question_: Are there any other client features that are obviously important for experimenting with plugins?


 
## Plugin Lifecycle

Depending on its invocation scope, a plugin will receive certain notifications and requests from the editor automatically.

### Invocation Lifecycle

Invocation-scoped (one shot) plugins have the simplest lifecycle. In general they will be invoked in response to a specific user command (say, to render a markdown preview) or an editor event, such as the saving of a file. There are two possible approaches to the API, which are very similar: in the first, after the RPC channel is opened, the plugin receives a `run` request, which will contain basic information about the invoking buffer, as well as the command responsible for the invocation.

The second major component of `run` would be an `invoking_method` or similar field, which is a method/params pair representing the request or notification that caused the plugin to fire. A single plugin executable should be able to define multiple triggers that it responds to. 

The alternative is to have their be two commands: an `initialize` notification followed by some `request` method. This has the advantage of standardizing the `initialize` event across all plugin categories. I prefer the first approach: I like that to the application and to other plugins, there would be no distinction between calling a method on a running plugin and calling a method that invokes a single-shot plugin. 

possible example `run` rpc:

```json
"method": "run",
"params": {
    "buf_info": {
        "buf_len": 1337,
        "revision": 161,
        "syntax": "rust",
    },
    "invocation": {
        "method": "xi-python-pep8.command.format", # or some other way of namespacing a command
        "params": {},
    },
}
```

Once it has received the `run` request, the plugin is expected to perform its work and exit. 

### Editor-local lifecycle

Editor-local scoped plugins are associated with a given buffer, receive lifecycle events for that buffer, and can query that buffer through the plugin API.

_Question_: How will we handle multiple views per buffer? Are plugins buffer scoped or view scoped? It seems silly to be running a linter in multiple views into the same buffer, but certain use-cases (such as acting on selections, or showing the cursor position in the status bar) want to be running per-view. 

The lifecyle for editor-local plugins is more involved than for invocation plugins. They should receive some version of `initialize` and `deinit`, for doing setup and teardown. They should also receive all of the [`EditCommand`](https://github.com/google/xi-editor/blob/master/rust/core-lib/src/rpc.rs#L52) events from core (probably?) and they should receive the special `update` event which sends deltas representing changes to the buffer.

_Q_: Are there any other significant lifecycle events that would be useful for this alpha API?

_Q_: Should a plugin receive events by default, or should it have to register for notifications it is interested in? The first is simpler, but inefficient; message volume on the wire will grow linearly with the number of plugins.

_Q_: should plugins be able to register/deregister for notifications while running? If we're doing a registration-based system then yes, probably, at some point?


Other events to add, eventually:
- hover reporting, mouse position reporting;
- window resizing? window gain/lose focus? 

### Global lifecycle

Global plugins receive all of the events that editor-local plugins do, and for multiple open buffers (there should be some mechanism for indicating buffers of interest) plus they also receive all file_open/file_close events.

**n.b.**: file_open/file_close/new_file are currently a bit messy; there are no editor-level new_file or file_close methods at the moment, but arguably there should be.

_Q_: What else might be useful here?

_Q_: How are buffers identified to global plugins? Do we have some internal identifier (such as the tab identifer currently in use) or do we have some new identifier that is buffer specific, as opposed to being linked to a view?

## Plugin API

Once running, plugins are able to communicate with `xi-core` through the standard rpc mechanism. I'd like to flesh out this API a bit more, to provide a baseline set of functionality for people experimenting with plugins.

Much of the particulars of this API could in many cases will be abstracted behind a good client library: in python, for instance, a `Buffer` class might have a `lines` member which implements `__getitem__` and fetches raw bytes over rpc as necessary.

_Q_: How should the underlying RPC methods be namespaced? (Should they be namespaced?) This may be helpful for message routing, but also for debugging and legability. For the purposes of this document I am going to namespace things according to the struct where the given request would currently be handled:

### proposed bare bones API:


#### methods available to all plugins:
_global plugins will need to specify a buffer when calling methods, other plugins will not_.

**n.b.**  there's some conflation here right now between the API provided by the plugin library and the actual rpc protocol.

_Q_: by what interface should the buffer be presented by the plugin library?

#### requests
- `buffer/get_data(offset, max_size, rev)` (implemented): returns raw bytes.
- `buffer/path()`: returns the filepath, if the file is saved, else `Null`
- `buffer/syntax()`: returns the currently active syntax definition
    
    _Q_: should we just have a single `buf_info` call that returns a variety of basic information?
- `view/selections()`: returns a list of the current selections
- `view/cursors()`: returns a list of cursor positions
- `view/position_for_offset(offset)` convert a byte offset into a line/column pair
- `view/active_plugins()`(?? maybe not for this version of the API)
- `client/prompt(prompt_text, type?, options?)` prompts the user for input. Maybe can provide a type for input validation, or a discreet list of options (for selecting a syntax definition, e.g)

#### notifications
- `buffer/update(delta)`: attempts to modify the buffer
- `view/set_cursors(cursor_positions)`: sets the cursor(s)
- `view/set_selections(selection_ranges)`: sets the selections
- `view/set_fg_spans(span_info)`: (implemented) sets text styles
- `view/update_statusbar(status_item_id, message)`: updates the text on a status bar item
- `client/show_alert(message)` displays an alert dialog.

_Q_: how do buffers identify themselves, e.g. for open/close?

## Other things to think about:

### message passing / routing:
As plugins can define new RPC methods, the approach of representing all possible methods as enum members will have to be adjusted. 

This raises a larger architectural question about plugin method dispatch. Currently plugins interact with an `Editor` instance, but there's no mechanism for the editor to pass unknown methods to some other responder. 

In my initial prototyping, there was a `PluginManager` type that was owned by each editor, but this has some trickiness around global plugins. 

I'm currently thinking about an approach where there is a single PluginManager that exists at the level of the `Tabs` struct, and mediates all plugin communication, keeping track of which plugins are active and available for which `Editor`s. It could also forward relevant RPCs to relevant plugins, and the `Editor` wouldn't know about plugins directly, but (for methods like `update`) calls would be forwarded through `TabCtx` (maybe?) returning to the editor the count of revs_in_flight.

### misc

- Timing, profiling, timeout: it might be useful to think early on about building in support for profiling plugin performance. 
- Manifest format: this proposal intentionally avoids discussing a manifest format. For development, I would prefer to represent plugin manifests as Rust types, and then when there's a better sense of what those types look like a manifest representation can be derived from them.
- plugin settings and user settings, generally.
- API for standardized serialization locations.
- what is the actual API for statusbar stuff?

## further reading:
(mostly to look at API structure)

- [Sublime Text 3 API Reference](https://www.sublimetext.com/docs/3/api_reference.html#sublime_plugin.ApplicationCommand)
- [Atom Documentation](https://atom.io/docs/api/v1.15.0/PackageManager)
- [VSCode Extensibility Reference](https://code.visualstudio.com/docs/extensionAPI/overview)

