---
layout: page
title: Notes on writing front-ends
site_nav_category_order: 201
is_site_nav_category2: true
site_nav_category: docs
---

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

### First steps

When a frontend is initialized, the first thing it does is launch and connect to
the core. Currently, the core is always run as a process, although we expect at
some point it could be run in the client process as a library.

After establishing a connection with the core, the client sends the
[`client_started`](#client_started) RPC. Core will respond by notifying the
client of some initial state, such as a list of available themes. The client
then normally sends a [`new_view`](#new_view) request; when it receives
a response it can begin sending editing operations against that view.

### Additional Resources

This document is not always perfectly up to date. For a comprehensive list of
supported commands, the canonical resource is the source, specifically [rust/core-lib/src/rpc.rs](https://github.com/xi-editor/xi-editor/blob/master/rust/core-lib/src/rpc.rs).

- The protocol is described in
  [docs/frontend-protocol.md](frontend-protocol.md).
- The config system is explained in more detail in [docs/config.md](config.md).
