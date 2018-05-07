---
layout: page
title: Plugin architecture
site_nav_category_order: 202
is_site_nav_category2: true
site_nav_category: docs
---

**Note:** This document is mostly of historical interest. Although the
high level details remain true, implementation details have changed,
and remain in flux.

## Philosophy

All serious programming editors have a mechanism for extensibility,
often in the form of plugins, also often in terms of a scripting
language with bindings to editor objects.

Xi follows the philosophy that plugins should be asynchronous, and it
should be possible to write them in any language. Thus, plugins are
invoked through RPC, and no language bindings are provided inside the
front-end or back-end process. A slow plugin should not interfere with
typing or other editing operations, and a crashing plugin should not
lead to data loss.

The idea of asynchrony is appealing, but actually implementing it is
challenging. The fundamental problem is that the plugin may provide
edits (for example, inserting indentation in a language auto-indent
mode) that happen at the same time as additional edits by the user.
These edits must be reconciled somehow. In some cases, specificially
where the user edits are to the text and not the rich-text annotations,
and the plugin edits are to the annotations but not the text (as will
often be the case for syntax highlighting), there is no fundamental
conflict and the overall system will eventually converge. (In this
case, it converges to the same state as running the plugin in batch
mode over the text input, and this should be taken as a correctness
criterion for plugins that do incremental computation). In xi, I plan
to address the challenge of parallel edits head-on, using some form
of operational transforms or differential synchronization. One
potential advantage of this approach is that it may make collaborative
editing practical.

In xi, not everything is a plugin. Even though the front-end to
back-end communication is similarly mediated by RPC, the protocol is
quite different.

It is expected that most plugins will be written on top of a
convenience library. This library will provide caching for access to
buffer contents, and in general abstract away low-level details of the
RPC protocol. Even so, it should stay small and simple, so it is
practical to provide for a number of languages. Initially, I will
probably develop the plugin protocol and library using Python for
rapid iteration, and then do Go and Rust, for higher performance.

Xi will not provide a package manager, but rather will defer to
existing mechanisms. Using, for example, apt-get, brew, or chocolatey
to install plugins should work well. Alternatively, others may choose
to package a _distribution_ containing the xi editor (front-end and
back-end) along with a curated collection of plugins, as, for example,
[Anaconda](https://www.continuum.io/downloads) does for IPython.

## Basic architecture

### Invocation; config files

Deciding when to invoke a plugin, and how, is non-trivial. This starts
with a configuration file, which represents a _trigger_ of when to
invoke the plugin, as well as a path to the plugin and some options. A
trigger can be a keyboard and/or menu command, a programming language
(so basically a selector based on file extension), or hooks for other
events (an example would be running gofmt before every save).

There are three levels of invocation: one-shot, per-buffer, and
editor-global. In a one-shot invocation, the editor starts the child
process, performs the RPC, and shuts down the process when the RPC
is complete. In per-buffer invocation, the process stays open for the
life of the buffer. If more than one buffer requires the use of a
plugin, xi will invoke multiple instances. In editor-global, a single
process is expected to accept RPCs involving multiple buffers, and
RPC requests are annotated with the buffer id.

The config file also indicates the protocol version expected by the
plugin, and xi will attempt to conform to a range of versions in
actual use.

Loading plugin info potentially has huge impact on startup time. Xi
will load all config files at startup, but will attempt to defer
executing the plugin binaries. Thus, the format of the config files
needs to be quick to parse. I am leaning toward TOML as providing a
good balance. (YAML is an alternative, I'm considering it because it's
required to process new-format
[Sublime Text syntax definitions](https://www.sublimetext.com/docs/3/syntax.html).)

For developing plugins, the config file can direct the plugin to be
compiled at invocation time ("go run" or "cargo run", for example).
This mechanism should not be used for publishing plugins and
distributing them to users.

I'm thinking that config files will be able to to "include" another
one. This would be the preferred way to handle optional plugins; they
would be stored in a directory that would not by default get processed
on editor startup, but a config file in a user-editable space can
point to another one.

### Read access to the buffer

When attaching a buffer (ie, on startup of one-shot or per-buffer
plugins), xi starts by sending a one-megabyte window of the buffer,
centered around the cursor. The plugin may request additional
substrings of the buffer through RPC. Note that such requests access
a _snapshot_ of the buffer, even if the user is concurrently editing.
As mentioned above, buffer access is one of the functions provided by
a convenience library - the actual plugin logic should be able to
request an arbitrary substring, or iterate through all lines, and have
that served by cache and RPC on cache miss.

When the RPC to the plugin completes, the snapshot is released. Any
edits to the buffer are then sent as _deltas_ to all plugins
subscribing to that buffer. Again, a major function of the convenience
library is to apply these deltas. The deltas may also, of course,
trigger computation, such as reapplying syntax coloring.

### Write access to the buffer

The plugin can also send deltas back to the core, either in the course
of RPC processing or spontaneously. These deltas can be to the text
buffer (for example, for indentation and electric brackets) and as
rich text spans (for syntax highlighting).

These deltas are suggestions; the core may need to reconcile them with
other edits, and may possibly discard them. Xi will communicate back
to the plugin to indicate whether the delta was accepted as-is or
modified. A sophisticated plugin may attempt to retry, based on more
up-to-date information about edits to the buffer. This seems like a
reasonable approach to implementing differential synchronization.

Other responses from the plugin are expected to include:

* Populating a completion menu.

* Displaying status messages.

* Popping up modal dialogs?

* What else?

### Asynchrony modes

Three asynchrony modes are anticipated. I might not implement all of
them.

In synchronous mode, additional edits to the buffer are blocked until
the RPC to the plugin completes. Thus, the deltas produced by the
plugin are applied as-is, with no possibility of conflicts from
concurrent edits. This is the simplest mode, but discouraged because
it can cause typing lag.

The normal mode is described above. During the life of an RPC, the
plugin operates on a read-only snapshot of the buffer. No further
deltas are sent to the plugin until the RPC completes. At that point,
the xi core merges any resulting deltas with the other concurrent
edits, and sends the plugin a notification of how the deltas were
resolved. In this mode, much of the asychnronous nature is hidden from
plugins; simple plugins can simply trust the core to reconcile the
deltas correctly, and take no further action

In fully asynchronous mode, deltas are sent from the core to the
plugin as soon as editing operations are made. My current thinking is
that each delta introduces a "generation number," and that queries
to retrieve buffer contents reference a specific generation number.

The distinction between normal and fully asychronous modes may be
implemented simply as a choice of what the plugin chooses to do with
delta notifications - if it batches them up until the RPC completes,
then it is effectively normal mode. This also seems like a good
function for the convenience library. Synchronous mode, however,
requires explicit cooperation from the editor, to prevent concurrent
edits while an RPC is in flight.

## Security

Plugins can potentially

## Open questions

The state of the art technique for syntax highlighting is to store an
explicit highlighting state at the beginning of each line (in general,
this state consists of a stack of begin/end nested rules; it is in
theory unbounded but in practice will take on a small number
of values). The fundamental syntax highlighting step, then, is a
function that takes the line state and the text of a line, and
produces a set of rich text spans for the line, as well as the line
state for the beginning of the next line.

A large part of the convenience library will be geared to efficient
incremental computation based on these primitives. A key observation
is that, when processing a delta, if you reach the same line (after
the parts changed by the delta) in the same state, you can stop
processing; all subsequent highlighting will be untouched. Of course,
typing `/*` can cause a state change to cascade to the end of a
document (this is one reason many "electric" modes auto-insert a
closing `*/` to try to keep such things balanced).

The open question is: where should the state be stored? I'm leaning
to do it in the plugin, but a case can be made for letting the core
function as a "database" that can store this information efficiently
even for huge files. Note though, the state info can be considered a
cache, because it is always possible to reconstruct it by scanning
from the beginning of the buffer.

### Additional use cases

Many of these use cases are ambitious, requiring sophisticated UI
wiring; they are unlikely to be implemented soon but might be worth
thinking about.

* Access to version control (maybe including display of diffs, or,
  more ambitiously, providing UI to do interactive merging).

* Embedding in a debugger (just annotating breakpoints and the like
  should be fairly straightforward).

* Source code navigation, including reference hierarchies.

## References

### Other editors

* [Neovim](https://github.com/neovim/neovim/wiki/Plugin-UI-architecture).
  Asynchronous RPC-based. GUI front-ends are another form of plugin.

* [Sublime Text](http://www.sublimetext.com/docs/api-reference).
  Basically exposes editor objects (views, windows, regions, etc)
  through Python bindings.

* [Vis](https://github.com/martanne/vis#lua-api-for-in-process-extension).
  Lua bindings for in-process extension, used for syntax highlighting
  (which is also PEG-based).

### Editor prototypes

* [Swiboe](https://github.com/swiboe/swiboe#swiboe---)
  ("**Swi**tch**bo**ard **e**ditor"). Everything is a plugin.

* [Wi](https://github.com/wi-ed/wi). Fully asychronous, written in Go.

### Fundamental technology

* [Differential Synchronization](https://neil.fraser.name/writing/sync/).

* [Operational transformation](https://en.wikipedia.org/wiki/Operational_transformation).

* [JSON-RPC 2.0 Specification](http://www.jsonrpc.org/specification).
