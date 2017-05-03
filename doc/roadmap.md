This is a basic roadmap for xi editor. It does not represent a commitment, or a strict ordering of when features will be done, but does outline the current thinking. As a community-driven project, contributors should feel free to pick up a desired feature and make it happen.

While development is ongoing, the MacOS/Cocoa front end (at [xi-mac](https://github.com/google/xi-mac)) is considered the "reference implementation". Contributors are welcome (and encouraged) to play around with other frontends, but various aspects of the RPC protocol are going to be in flux for the forseeable future. The Fuchsia front end (at [fuchsia/xi](https://fuchsia.googlesource.com/xi)) is also officially maintained, but only has a subset of the capabilities.

## Roadmap
### next/alpha/towards a useable editor (summer 2017?)
The immediate goal is to get xi to a point where it is a suitable choice for basic text editing, and to enable development of basic plugins.

- [ ] automatic linewrap (#184)
- [ ] horizontal scrolling (https://github.com/google/xi-mac/issues/4)
- [ ] simple find/replace (https://github.com/google/xi-mac/issues/9, #150)
- [ ] automatic syntax highlighting (inc. automatic syntax detection) (this comes down to improved plugin support)
- [x] warn on close with unsaved changes (#174)
- [ ] preliminary plugin support (#189)
- [ ] Goto-line ? (#190)
- [ ] multiple cursors (#188)
- [ ] find / replace (#224)

### near future
Things that are definitely planned

- [ ] stabilization and good documentation of front-end protocol
- [ ] multi device editing (may be Fuchsia-only at first)
- [ ] making the syntect plugin properly incremental
- [ ] multiple views into a single buffer
- [ ] split windows (#170)
- [ ] remapping keys
- [ ] more customization, generally
- [ ] full plugin support (inc. language server protocol)
- [ ] better i18n and Unicode support (including bidi)

### future
(one day)

- [ ] modal editing (vi mode) (#93)
- [ ] rich text (similar document model as Markdown)
- [ ] some form of workspace, or navigation through multiple files (#181)

### deep future
(who knows)

- [ ] collaborative editing
- [ ] remote sessions
- [ ] [crispr integration](https://xkcd.com/1823/)
