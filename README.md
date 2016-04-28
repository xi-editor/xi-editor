# xi editor

<img src="icons/xi-editor.png" alt="Xi Editor" width="256" height="256"/>

The xi editor project is an attempt to build a high quality text editor,
using modern software engineering techniques. It is initially built for
Mac OS X, using Cocoa for the user interface, but other targets are planned.

Goals include:

* Insanely high performance. All editing operations should commit and paint
  in under 16ms. The editor should never make you wait for anything.

* Beauty. The editor should fit well on a modern desktop, and not look like a
  throwback from the ’80s or ’90s. Text drawing should be done with the best
  technology available (Core Text on Mac, DirectWrite on Windows, etc.), and
  support Unicode fully.

* Reliability. Crashing, hanging, or losing work should never happen.

* Developer friendliness. It should be easy to customize xi editor, whether
  by adding plug-ins or hacking on the core.

Screenshot (will need to be updated as syntax coloring and UI polish is added):

![xi screenshot](/doc/img/xi-mac-screenshot.png?raw=true)

## Getting started

You need Xcode 7 and [Rust](https://www.rust-lang.org/) (version 1.8 is
recommended). You should have `cargo` in your path.

```
> git clone https://github.com/google/xi-editor
> cd xi-editor
> xcodebuild
> open build/Release/XiEditor.app
```

Or `open XiEditor.xcodeproj` and hit the Run button.

### Building the core

If you’re not on a Mac, you can build just the core:

```
> cd rust
> cargo build
```

See the [xi_glium](https://github.com/potocpav/xi_glium) project for an
experimental front-end in Rust.

## Design decisions

Here are some of the design decisions, and motivation why they should
contribute to the above goals:

* Separation into front-end and back-end (the latter also known as “core”)
  modules. The front-end is responsible for presenting the user interface and
  drawing a screen full of text. The back-end holds the file buffers and is
  responsible for all potentially expensive editing operations.

* Native UI. Cross-platform UI toolkits never look and feel quite right. The
  best technology for building a UI is the native framework of the platform.
  On Mac, that’s Cocoa.

* Rust. The back-end needs to be extremely performant. In particular, it
  should use little more memory than the buffers being edited. That level of
  performance is possible in C++, but Rust offers a much more reliable, and
  in many ways, higher level programming platform.

* A persistent rope data structure. Persistent ropes are efficient even for
  very large files. In addition, they present a simple interface to their
  clients - conceptually, they're a sequence of characters just like a string,
  and the client need not be aware of any internal structure.

* Asynchronous operations. The editor should never, ever block and prevent the
  user from getting their work done. For example, autosave will spawn a
  thread with a snapshot of the current editor buffer (the peristent rope
  data structure is copy-on-write so this operation is nearly free), which can
  then proceed to write out to disk at its leisure, while the buffer is still
  fully editable.

* Plug-ins over scripting. Most text editors have an associated scripting
  language for extending functionality. However, these languages are usually
  both more arcane and less powerful than “real” languages. The xi editor will
  communicate with plugins through pipes, letting them be written in any
  language, and making it easier to integrate with other systems such as
  version control, deeper static analyzers of code, etc.

* JSON. The protocol for front-end / back-end communication, as well as
  between the back-end and plug-ins, is based on simple JSON messages. I
  considered binary formats, but the actual improvement in performance would
  be completely in the noise. Using JSON considerably lowers friction for
  developing plug-ins, as it’s available out of the box for most modern
  languages, and there are plenty of the libraries available for the other
  ones.

## Current status

This is still a project in its early stages. The Mac build has basic editing
functionality (it was used to write this README), but looks very spare and
is still missing essentials such as syntax highlighting and auto-indent. At
the moment, it’s expected that its main community will be developers
interested in hacking on a text editor.


## Authors

The main author is Raph Levien.

## Contributions

We gladly accept contributions via GitHub pull requests, as long as the author
has signed the Google Contributor License. Please see
[CONTRIBUTING.md](CONTRIBUTING.md) for more details.

### Disclaimer

This is not an official Google product (experimental or otherwise), it
is just code that happens to be owned by Google.
