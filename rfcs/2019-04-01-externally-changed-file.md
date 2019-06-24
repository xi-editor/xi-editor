# RFC: Acting upon an externally changed file

Right now Xi only has two mechanisms to handle externally changed files:

* Blocking saving, instead requiring the user to save the file elsewhere
* Automatically reloading the file if no changes have been done

Blocking saving is a bit annoying, especially if the user _does_ want to overwrite
the file (e.g. when a git rebase has modified a file's modtime).

## Summary

For this to work we have to change multiple thing:

- Add a new notification ('file_externally_changed')
	- Frontends may use this to display a 'File has been changed, want to reload?' dialog, but this is not mandatory - they can warn on save, too.
- Add a way for frontends to force reload the file to implement the previously mentioned dialog
- Make 'save' a request, possible under the name 'save_req', or ' try_save', since saving can fail (due to the file having changed or other means). Add a switch to this to make it possible to overwrite an externally changed file

### file_externally_changed

This needs the `notify` feature of xi-core enabled. Upon detecting a file change we'd simply send the frontend a `file_externally_changed { "view_id": id }` upon the frontend
can do two things:

* Ignore it (and handle external file changes during save)
* Act on it, e.g. by showing the user a dialog with "reload | ignore".

For the latter we'll also need another RPC for the frontend to request reloading
the file, e.g. `reload { "view_id": id}` -> "Result", which will force Xi to reload the file.
Since this may fail (e.g. if we don't have the required permissions to read the
file anymore), this should be a request.

### Making save a request

Since file changes are prone to failing (disk in ro-mode, not the required permissions
to write to the file and so on), `save` should be a request, possibly under the name
`save_req`, or `try_save`. It should return something similiar to alert, so a statuscode
(0 meaning "success", non-zero values being different error code) and an Option<String>
for an err_msg, if saving has failed:

`try_save {"view_id": "view-id-4", "file_path": "save.txt", "overwrite":" false"} -> Result<usize, Option<String>`

* overwrite is a new param, to force overwrite a file, even if externally changed

### Implementing this in frontends

There are multiple ways to tackle this:

The frontend can display a save dialog like "The file has been changed,
do you really want to overwrite it?", if `try_save` returns an error code
that the file has changed externally.
	- Nano does that. First you save via Ctrl+O, after pressing enter the
	  following dialog comes up:
	  ![nano reload](./assets/nano_reload.png)

2.
The frontend can display a dialog once `file_externally_changed` has been received,
e.g. VSCode and VSStudio do this.

![vscode reload](./assets/vscode_reload.png)
![vsstudio reload](./assets/vsstudio_reload.png)
