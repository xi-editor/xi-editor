
## Motivation

For an overview of the general problem, see [#645](https://github.com/google/xi-editor/issues/645). For an earlier RFC on a similar problem, see [#374](https://github.com/google/xi-editor/blob/plugin-tag-rfc/doc/rfcs/plugin-lifetimes.md). To quickly recap: we would like plugins to be able to add support for new languages, but we do not want to put any constraints on how that support is implemented; and we would like some mechanism by which we can ensure that only a single plugin is providing core language support for a given language.

## Proposal

### A plugin can add new language definitions in its manifest

In xi, support for a new language is added by creating a plugin that includes a language definition in its manifest. A language definition contains exactly enough information to assign files to a language; this will be similar to the [language definitions in VS Code](https://code.visualstudio.com/docs/extensionAPI/extension-points#_contributeslanguages).

This declaration is independent of adding any actual support for the language, although in the general case a plugin which is declaring a new language is probably also providing some language features.

### A plugin can declare itself a 'language provider'.

Alongside declaring a new language, a plugin can declare itself as providing _support_ for that language. For a given buffer, at most _one_ 'language provider' plugin can be active at a time. This plugin is solely responsible for the following features:

- syntax highlighting
- auto-indent + autoclosing braces
- comment toggling
- identifying code folding regions
- ??

The plugin may _also_ provide support for other features such as autocomplete or hover definitions, but those can also be provided elsewhere; the features above should _only_ be provided by a given buffer's designated language provider.

### A language provider plugin can declare its priority

To resolve conflicts between multiple plugins providing support for the same language, plugins can indicate their priority. This may be as simple as a boolean 'is_specialized' flag; the main case is distinguishing between a general purpose plugin such as syntect and a more specific plugin targetting a particular language.

The case where multiple plugins provide equally specialized support for a language is considered user error; we should pop an alert and use one or the other.

### You can have a plugin that is _not_ a language provider that still provides features for a particular language.

If you want to have a plugin that syncs your notes across machines, or a plugin that renders a real-time preview of some markdown you're writing, these plugins can all run, along with a designated _language provider_, for markdown files. They are just expected to not also be trying to handle autoindent or syntax highlighting. (They might provide other sorts of annotations, though).

### Plugin activation and lifetime

When a buffer's language changes, xi-core looks for a plugin that can provide appropriate language support. If one is found it is launched (if necessary) and is bound to the new buffer. When that buffer closes or the langauge changes the plugin is unbound; a plugin that is not bound to any view shuts down.

In addition to being bound based on current language, a plugin can also be bound to a view by the user, by _itself_ (for instance after being launched by a command), or (probably) by another plugin.

In all cases, however, the plugin will be shutdown when it is no longer bound to a view.

Q: what's to distinguish being activated because we're a provider from being activated anywhere?
are `onLanguage: python` and `asLanguageProvider: python` two different activations? 

Q: If a plugin can be a language provider but also offers some other stuff, and we start it when it _isn't_ a provider, how do we let it know?

### Scope tags or language identifiers?

A major influence for this proposal is [#374](https://github.com/google/xi-editor/blob/plugin-tag-rfc/doc/rfcs/plugin-lifetimes.md). Most of the items mentioned in the 'Motivation' section of that RFC are either solved with this proposal or no longer apply (for instance plugins are now global only) but a question remains about whether buffer 'tags' should take the form of freeform syntect scopes ('text.markdown', 'pluginName.by.user', 'pluginName.by.anotherPlugin') or if they should be some kind of Enum, e.g.

```rust

enum Binding {
    OnLanguage { langauge_id: String},
    Manual { target: String, source: String },
    // ...
}
    
```

One downside of the scope approach is that it introduces the possibility of a file ending up with multiple syntaxes (a solution to this is outlined in [#374](https://github.com/google/xi-editor/blob/plugin-tag-rfc/doc/rfcs/plugin-lifetimes.md)). In return it gives us a lot of flexibility, and specifically allows us to elegantly handle the idea of 'sub-langauges', for instance allowing `text.html.ruby` to bind the plugins that `text.html` would, in addition to plugins that offer features specific to ERB.

A possible compromise would be to use scopes under the hood, but to only allow certain scope prefixes (such as `text` or `source`) to be added by xi-core, or through specific APIs.

Using scopes as language identifiers also lets us to do things like having language settings that inherit defaults from another language, e.g. `text.markdown` could inherit the defaults from `text`.

### Interaction with syntect

Exactly how this will interact with syntect remains up in the air, but I'll try to bullet-point my current best guess:

- Clients that bundle syntect will add a `syntect_packages` directory to the user's `config_dir`.

- To add a new language/syntax, the user will place `.tmLanguage` or `.sublime-syntax` packages in this folder.

- When syntect launches, it will check this path for new packages; if they exist it will read them, and will regenerate its own manifest to include the newly added languages

- Either the user will have to restart the editor, or we will add some 'needs_reload' plugin->core RPC to indicate that core should reload the manifest/plugin.

- language-specific default settings will move from core to the syntect plugin (default settings should be provided by the plugin that declares the language)

- syntect will need to gain the ability to handle `.tmPreferences` files, in order to support comment toggling and auto-format.

## Conclusion

This proposal was written without writing a lot of code, which means it is definitely subject to change. Some things definitely still feel pretty unresolved, but I hope it's at least a reasonable starting point.