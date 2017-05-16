
## Summary

A major feature of plugins is the ability to make new commands available to the user. This RFC outlines a preliminary system for describing these commands.

Thank you to @rkusa for the discussion and feedback that has been incorporated into this RFC.

### Notes on UI and implementation

The _particulars_ of how a given Xi client might make commands available to the user are beyond the scope of this RFC, but deserve their own discussion at some future point. For now, we will presuppose that a client has _some_ mechanism for letting the user select a command (perhaps by entering it manually at a prompt, or selecting it from a menu or a command palette) and also that there is some mechanism for soliciting user input.

### Basic implementation

At its most basic, a command is an RPC method + some metadata. As an example, let's suppose we have a plugin that wraps [`rustfmt`](https://github.com/rust-lang-nursery/rustfmt), a tool for formatting Rust code. We would like to provide two functions: one which runs rustfmt on the current buffer, and one which previews changes, showing them in a new window. Neither of these functions takes any additional arguments; they both operate on the buffer open in the active view. As json-rpc, these might look like this:

`{"method": "rustfmt.run", "params": {}}`
`{"method": "rustfmt.preview", "params": {}}`

To make this more usable, we would flesh this out with some additional information that the client can use when presenting the command:

```json
{
    "title": "Rustfmt: run",
    "description": "Run rustfmt on the current buffer",
    "rpc_cmd": {
        "method": "rustfmt.run",
        "params": {},
        },
}
```

This might be extended to include additional information, such as optional display information (say a preferred icon), or a list of tags or keywords.

### Command catalog

As part of view initialization, the view is sent a list of available commands. I will refer to this list as the _command catalog_. This catalog may be updated at various lifecycle events, for instance if the buffer's syntax definition changes. The client is expected to provide the user with the ability to manually run any of the commands available to the currently active view.

### User input

Some commands may require user-supplied arguments. In general, I can think of two types of arguments: raw input and constrained selection. Raw input requires the user to manually enter some value; constrained selection presesnts the user with a list, and requires them to select an item.

I propose a similar approach to both of these situations: the `command` gets an `"arguments": []` field, each item in which carries some tag, which corresponds to an item in the `rpc_cmd.params` object. In the case of a selection, the argument item contains a list of options, similar to the top-level `command`. In the case of raw input, the argument is a tag, an input type, and possibly a placeholder value.

```json
{
    "title": "Git: rename branch",
    "description": "Renames the current branch",
    "rpc_cmd": {
        "method": "git.rename_branch",
        "params": {
            "new_name": "{new_name}"
            },
        },
    "arguments": [{
        "tag": "new_name",
        "type": "String",
        "placeholder": "new branch name"
    }]
}
```

In this example, the rename command requires the user to provide a new branch name. After the user selects the command, the client checks if command expects any arguments; if it does, the client prompts the user for each argument, validates the input, and then assembles and sends the specified RPC.

#### Input types

What types of input do we expect? The values that seem immediately obvious are `Number`, `Int`, `PosInt`, `Bool`, `String`, and `Choice`. Some less obvious options include `File` (some existing file), or `Url`. Additionally, there could be a special case where validation is performed by the plugin itself.

These all expect raw input, except for `Choice`, which would include an `"options"` field:

```json
...
"arguments": [{
    "tag": "new_theme",
    "type": "Choice",
    "options": [
        {
            "title": "Base16-ocean (Dark)",
            "value": "base16-ocean.dark"
        },
        {
            "title": "InspiredGithub",
            "value": "InspiredGithub"
        },
        ...
    ]
}]
```

In this case the client would be expected to present some UI which allows the user to select one of the `option`s, and use that to construct the command.


### Plugin initiated user input

In some cases, a plugin may need some user input without knowing ahead of time, or without knowing what the relevant options might be. For instance, a Git plugin might want to provide a wrapper around `git add`, and allow the user to select a modified file. In this case, the plugin would be responsible for getting the user input, using some `get_user_input` rpc method; that method would provide an `arguments` list as in the above value, with the user's input returned to the plugin as a mapping of tags to values.


There may be an argument that we could use this mechanism for _all_ plugin commands; that is, the command would be sent to the plugin, which would then call out to the user and collect the various args. This approach has a few drawbacks, however: it will make commands harder to reuse in inter-plugin communicaiton (plugins won't be able to just call each other's methods, but will additionally have to do argument negotiation as a series of RPCs)), it will increase plugin boilerplate, and it will make plugin manifests less self documenting, in that a command description won't necessarily enclude argument info.

## Discussion

Are there any clear limitations to this approach? Are there any unsupported use-cases? Are there any additional concerns?

