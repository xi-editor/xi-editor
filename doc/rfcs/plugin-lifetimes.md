## Motivation

The current/earlier system/plan for handling plugins has a number of problems:

- The two-level tree in `PluginManager` is kinda hairy. This can be fixed by #371
- There is no way to have a plugin process that is shared between views but also scoped. For example if I write a plugin for Rust autocomplete, I can either have it spawn a 100 processes for 100 Rust files, or spawn a process regardless of if I have any Rust files open. This is seriously bad, if my Sublime did this it would have ~100 processes.
- Languages have to be specified in the core.
- It's not clear how global commands fit in, especially ones that start views, for example like the GitSavvy Sublime plugin which has commands that open special views. In the Xi model this would require a global plugin, regardless of if you use the command or open any views.

I propose the following design as a solution to all of these concerns, it works best after or together with #371.

## Design

### Buffer Tagging

Instead of buffers having a defined syntax, they have "tags" which are a `BTreeSet<syntect::Scope>`.

There should be an API for both the frontend and plugins to add, remove and perhaps atomically compare and replace tags. These might take the form of tag "transactions" with a set of inserted tags and removed tags, either of which may be empty, the transaction only runs if all the tags to be deleted are present. This will prevent asynchrony issues allowing for example, a file to end up with two syntax-related tags.

### Syntaxes

Plugin manifests may define syntaxes that they provide, these specify a human-readable `name`, a `scope` (e.g `source.php`, `text.markdown`), and a list of extension strings. When a file is loaded with one of the given extensions it will be tagged with the given scope plus the id of the plugin appended as an atom, for example `source.emacs-lisp.syntect`.

This allows disambiguation for example if `syntect` and `fast-rust` both provide highlighting for Rust, it's possible to determine which one should highlight a buffer, and possibly provide a UI for choosing which does it, with the default determined by plugin priority/order (a concept we need for other things).

### Buffer Selectors

The use of scopes allows searching for buffers with varying specificities with selectors, for example "" matches all buffers, "source" matches all code files, "text" matches all markup/text files, "source.rust, text.html" matches all Rust and HTML files regardless of what is highlighting them, "text.html.ruby.syntect" applies only to Rails templates highlighted by syntect, "text.markdown plugin.simplenote.synced" matches markdown files that have also been specially marked for syncing by the `simplenote` plugin.

Plugin manifests can define a [scope selector](https://manual.macromates.com/en/scope_selectors) (which can use commas to match multiple things, or maybe this could be just a list of selector components) that they *bind* on. This is how lifetimes work, which will be described later. Plugins also automatically bind on the syntaxes they have defined (including the included plugin name), as well as `plugin.the-plugin-id.enabled`.

### Plugin lifetimes

The core process has a set of currently running plugins. Every plugin (as specified by manifests), is either running or not running.

Whenever the tags of a buffer change, which includes when a buffer is created or deleted, all bindings are updated. Any plugins that did not previously bind on a buffer are sent a notification about the newly bound buffer. Any plugins that were binding on a buffer but no longer should (either a tag was removed or the buffer was closed.), are sent a message that a view was unbound.

If a plugin is bound on a view but isn't running, or a command defined in the plugin's manifest is run, that plugin is started.

When a plugin's last bound view is unbound and all commands it is running are complete (TODO: we may need another mechanism for knowing this), stop the plugin. Note that buffers may be bound while a command is running, so a global command from the plugin can create a buffer that it binds on which keeps it alive.

All plugin notifications for a buffer are only sent to plugins bound to that buffer.

### Conclusion

I think this design solves a lot of problems in one simple-ish mechanism:

- Binding syntax highlighting plugins to extensions and being able to disambiguate them
- Binding language-related plugins in a good way, including for specialized languages. For example both Emmet (HTML) and my-rails-template-helper (Rails ERB-specific) will activate on `text.html.ruby`.
- Enabling plugins for specific buffers, a frontend just has to tag it as `plugin.the-name.enabled`.
- Special plugin views, for example I can have a plugin that syncs specific markdown buffers that it creates without having to start on all markdown documents.
- It only creates at most one process for each plugin, preventing process bloat.
- Plugins don't have to change their code when they change or extend how they start up, only the manifest.

### Alternative

One problem with a lot of existing editors is that for example, Javascript/CSS completion doesn't work in CSS/Javascript embedded in HTML, even if highlighting does.

Instead of using a separate tag set on the buffer, binding could use the scopes present in a document from syntax highlighting. Allowing a javascript autocomplete plugin to handle embedded JS. It's also one less piece of state.

There's a couple problems that make this not my primary suggestion:

- This makes it harder to solve the problem for syntax highlighters.
- Autocomplete/formatting/etc plugins will have to be aware of embedding to work properly anyway, if you try to run a JS formatter on an HTML document with embedded JS it will syntax error. And if a plugin does know how to handle it, it can just bind on `text.html` as well.
- It's harder to make efficient.
