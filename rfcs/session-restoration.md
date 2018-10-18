## Motivation

As per [October 2018 Roadmap](https://github.com/xi-editor/xi-editor/issues/937), the focus now
is on making xi-editor viable for daily use.

To be viable for daily use, an editor must be able to prevent data loss. This feature would also
allow for better continuation between closing and opening the editor, as it would allow the user
to reopen files just as they left them when closing the editor.

See also: [#913](https://github.com/xi-editor/xi-editor/issues/913).

## Proposal

### Overview

Upon initialization, the core creates a `Session` object which is responsible for storing [session
data](#structure-of-session-data). This object should be updated on
[every relevant event](#when-to-update-the-session) and it should serialize (to JSON?) and
**atomically** persist data to disk every 1 second (to be discussed).

Session data is internally represented as a `HashMap<ViewId, ViewData>`.
Session data is written to a `xi-session` file in a OS-specific directory (for macOS, that would
be `~/Library/Application Support/xi-core/`, along with the log file).

The frontend should be able to opt out of this feature. TODO example use case

### Structure of session data

For the first version of this, we will store a list of currently open views. Data for each view
includes:

- Absolute rope delta between pristine (last saved) and current states.
- Selection state
- Viewport position
- Configuration overrides
- Language
- File path, if present

### When to update the session

In terms of client RPC, session should be updated on the following actions:

- `new_view`: the new view gets inserted into the session.
- `close_view`: the view gets removed from the session.
- `save`: absolute rope delta gets updated for the view.
- `set_language`: the view language gets updated.
- `modify_user_config`: the user config gets updated.
- `edit`: depending on the actual operation, absolute rope delta, viewport position, or selection state gets updated.
- `quit` (new RPC): when the frontend quits, the core should persist the session data immediately.

### How to restore the session

After `client_started`, the frontend might receive a new `session_found` RPC. The frontend might then
send a new `restore_session` RPC, and the core should then go through the following procedure:

1. Deserialize session
2. Open all views that were opened in the session
3. Apply view data for each view

The frontend then gets notified about every action it is required to perform via core->frontend RPC
(we might need to extend it).

### Some notes

There should only be one saved session possible (am I missing something here?)
