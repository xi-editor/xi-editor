---
layout: page
title: GSoC
site_nav_category_order: 300
is_site_nav_category: true
site_nav_category: gsoc
---

List of project ideas for Google Summer of Code 2018:
 - [Xi-Mac Polish and UI improvements](#xi-mac-polish-and-ui-improvements)
 - [Language Server Plugin](#language-server-plugin)
 - [Improved Theming Support](#improved-theming-support)
 - [Theme / Syntax Loading](#theme--syntax-loading)
 - [Diff / Merge](#diff--merge)
 - [Workspace + Git](#workspace--git)
 - [Vi / modal editing](#vimodal-editing)
 - [Find enhancements](#find-enhancements)
 - [Scriptable language bindings (C API)](#scriptable-language-bindings-c-api)
 - [Flush out unit tests](#flush-out-unit-tests)
 - [Memory map text buffers](#memory-map-text-buffers)

--------

## Xi-Mac Polish and UI improvements

### Improve the core editing experience
There is a bunch of upcoming work in xi-core that will require frontend support. Much of this work would be a fun project for someone interested in doing UI programming for the reference Mac frontend, written in Swift/Cocoa. This could involve developing new features, such as a *completion menu*, *status bar*, *command palette*, or *split view* support; there is also room for a bunch of polish, specifically around the handling of tabs and windows.

### Outcome
For the polish work, the outcome is an editing experience that reduces user frustration and fatigue. For the feature work, the outcome is an editor that is better able to support the full range of functionality expected of a modern programmer’s text editor.

### Difficulty
This work will be difficult in that it requires serious attention to detail, and a nitty eye for UI stuff, as well as patience in digging through documentation and sample code. It is not enormously difficult as a software engineering task, however.

--------

## Language Server Plugin

### Expand the plugin protocol to enable fuller featured plugins
Xi currently has a minimal plugin API, capable of handling a small subset of possible plugin applications. We would like to expand this to support a variety of common plugin use cases, including auto-completion, displaying status information, navigating between files, and more. Specifically, we would like the plugin protocol to be capable of supporting all the features provided by the [Language Server Protocol](https://microsoft.github.io/language-server-protocol/). A final goal of this work would be to write a plugin that mediates between xi and a given language server.

### Outcome
Be able to communicate with a language server, receiving various kinds of semantic information such as hover definitions, completion suggestions, and symbol resolution.

### Difficulty
This is not enormously difficult technically, but will involve small modifications to numerous areas of the codebase. The most difficult component may be the protocol/API design aspects.

--------

## Improved Theming Support

### Improve how much of xi can be themed
Xi currently uses textmate theme files, which provide a limited degree of customization to editor features outside of the text buffer. A theme should be able to control all parts of the editor like gutter font, menu fonts, background colors, etc; to this end we should describe a backwards-compatible ‘textmate +’ theme format that includes more attributes.  Theming is important to the usability of programming text editors.

### Outcome
When this project is finished, we will have a robust and flexible theme format, offering options suitable to a variety of UI paradigms and implementations. This should include ways of specifying colors for warnings, errors, and info lints, gutter styling, active line highlighting, and more.

### Difficulty
Easy to do, hard to do well. This is an important part of the editor, and a good design will be respectful of the various needs of different frontends.

--------

## Theme / Syntax Loading

### Improve the syntax highlighting support for Xi
Xi currently uses a plugin based on [syntect](https://github.com/trishume/syntect) for general theming and syntax highlighting. That plugin comes bundled with a small number of themes and syntax definitions, but there is currently no way of adding new ones. This project would add support for watching specified directories for new themes and syntax definitions, and loading them into syntect.

### Outcome
When this work is finished, the user should be able to place custom themes and syntax definitions in an appropriate place on the filesystem, and have those items automatically detected and loaded by syntect. Syntect should also generate new binary dumps including the new items, to avoid unduly impacting startup time.

### Difficulty
This is low-hanging fruit for core development.  We already support syntax files. The work that would need to be done is to define RPC mechanism for a front-end to inform syntect of the syntax file path & for syntect to integrate a filesystem watcher to scan for changes. A minimal design docs or maybe even just an IRC discussion would be necessary. Additionally adding more syntax files built-in would improve the out-of-the box experience.  

There is opportunity for the student to expand the scope of the work to improve the performance of loading the theme files by compiling them into a binary format & caching them.  Would require some diligence to detect stale cache entries (updated theme file) or uninstalled themes.

--------

## Diff / Merge

### OSS merge could use a lot of work
Right now the story for merge tools, let alone native ones, isn’t particularly great. The gold standard Kdiff3 hasn’t seen very active development and suffers from a number of bugs (some related to trying to be cross-platform, some just inherent bugs in its core algorithm). Adding support for xi to act as diff/merge tool has several benefits in that it’s a simple enough operation that people might use it even before xi is ready for use as an IDE, kdiff3 provides a decent template for the UX with clear improvements that could be made. The plugin architecture for xi means that we can support different automatic diff/merge algorithms (there’s been a lot of work in this field since kdiff3 started). For Xi itself this work would flush out the ability for having multiple views to the same file which is valuable as a regular text editor.  A built-in diff mechanism would also be useful from the perspective of offering the user the option to view a diff if the file contents change while there are unsaved changes (e.g. checkout).

### Outcome
Be able to open xi in 2-way diff or 3-way merge mode from git.  This would require adding split-view support to a document.  Xi-core supports this to some extent but there’s a bit of front-end work that would be needed to expose this (+ fix any issues within xi-core that have incorrect assumptions).  Improving how alignments are managed & ensuring the view doesn’t reset unnecessarily when changes are made would instantly make this a more productive tool than kdiff3 (which is saying a lot). Both mechanisms would allow for selectable diff/merge algorithm backends as there has been a lot of work in this area (semantic diff/merge, patience algorithm, etc) since kdiff3 started.  Algorithms might be provided by multiple plugins.

### Difficulty
The split-view work should be relatively easy core development work.  Diff/merge algorithms are available in several languages so it should be easy to either port it to Rust or write the diff plugin in a more expressive language like Python (at least to start).  Riskier explorator work involves incorporating manual alignments into the diff/merge algorithm, which only Kdiff3 appears to have attempted, & how to keep the split views consistent despite potentially radically changes to the alignments (which Kdiff3 does not do well).

--------

## Workspace + Git

### Step 1 for an IDE experience
A workspace plugin would enable project-based editing. This would provide a foundation for performing actions like building, running & debugging a target.  It would also provide various higher-scope tools (e.g. refactoring) with the knowledge necessary about how to perform those actions (what files are part of the compilation, where to look for documentation, etc).  
A git plugin would allow for operations like annotating lines with commit info and for viewing the diff of a particular commit. When the workspace plugin is loaded then there would also be status indications for untracked/modified status of project files.

### Outcome
A workspace plugin that supports Cargo projects so that we could self-host.  Supporting CMake would be great too. This requires front-end integration as well for providing an “IDE” view. IDE’s typically have a single window that shows the project files + a view to edit a window. Additionally, at startup there’s an ability to provide a selection screen of which project(s) to open.  
Git integration at a minimum would entail a blame option & an ability to view the changes introduced within that diff (like Xcode).

### Difficulty
This would be medium-difficulty core development work for the workspace. It would probably require having another entry-point in the front-end for an IDE view. It would reuse a lot of the core rendering code but the document management would probably be distinct. CMake & Cargo are two projects that would probably be fairly easy to integrate as the files that are part of Cargo are mostly deterministic from a static TOML file and CMake can export all its commands to a JSON file. It would require coming up with a thorough project structure model. For full completeness, consideration would have to be given to properly incorporate build.rs logic.  

Most of the git blame work would simply require getting it to render correctly & efficiently. The git plugin itself shouldn’t be difficult since there exist great Rust bindings and git already supports blaming a line-range which is what the front-end would ask the plugin to provide.

--------

## Vi/modal editing

### Implement more flexible & extensible event handling.
Xi currently offers a traditional GUI interaction model. The frontend is responsible for translating keyboard events into ‘edit events’, which are sent to xi-core. For some time we have been envisioning a more flexible system, that will modularize event processing, making things like vi/kakoune keybindings possible. This project involves working closely with the core team to develop a robust design, and then producing an implementation and documentation.

### Outcome
When this project is finished, it should be trivial for a user to turn on a basic `vi mode`; and it should be fairly easy for other developers to experiment with new event handling models using the developed API.

### Difficulty
This problem has some interesting edge cases, but good support will be available from mentors. A basic solution should be fairly easy, but actually implementing a new event handler is a non-trivial design challenge.

--------

## Find enhancements

### Add regex find, find/replace, etc
The find functionality currently only supports plain-text search within the current document. A proper editor supports a richer set of functionality:
 - Regex search.
 - Find/replace within document
 - Find + Find/replace within a directory
 - Find + Find/replace within workspace files
 - Filter for matching lines. Show only matching lines with a configurable amount of context
 - Multiple queries. Match any number of queries and highlight them uniquely.
All of these use-cases have received significant attention by the ripgrep project which has factored them out into standalone Rust libraries (as fast or significantly faster than other tools like venerable grep).  The workspace files integration would depend on the workspace plugin but significant development can be made even without it.  Filter & multiple queries would be features exclusive to xi not seen before in text editors (filter is available in command-line grep and multiple queries are kind of possible via regex search).  

An additional feature for supporting multiple queries (each query highlighted separately but the search acting in a uniform way) would be super useful addition that would be unique to xi (often useful for log files).

### Outcome
One or more of the features above to be added. Most of the work would be in the core but some front-end work would be required (e.g. in xi-mac) to provide the best possible functionality.

### Difficulty
There is a variety of levels of difficulty available, even within a work item.  For example, regex search is fairly easy but can be improved by colorizing regex groupings within a selection. Same goes for things like multiple queries; one could simply join the queries using the | regex operator but that would lose the ability for being able to mix’n’match plain-text and regex queries, might make coloring different queries in different ways difficult, etc. This would be a mix of core development (different find features), exploration (how to assign multiple colors to matches), & fun work (UI/UX experimentation/trial & error, figuring out color schemes that are accessible, etc).


--------

## Scriptable language bindings (C API)

### Provide an API for core data structures and actions, suitable for binding to scripting languages and GUI frontends
Right now all communication between core and front-end, as well as between core and plug-ins, is mediated by a JSON-RPC communication mechanism. We’ve been finding that there are common operations (maintenance of the line case, for example) that would be worthwhile to make available for many clients; these are also typically performance sensitive, so would benefit from implementation in Rust. The project involves creating C API’s for such tasks, designed specifically to be easily wrapped in FFI bindings from scripting languages and the languages most commonly used to write front-ends.  

Related work would be similar FFI bindings for the xi-rope data structure, so plug-ins running as threads in the same process could have more direct access to the document contents.

### Outcome
This work would improve performance and decrease the amount of work needed to build front-ends and plug-ins, as these would be able to access high-quality shared implementations of data access and actions.

### Difficulty
This is probably medium difficulty and not particularly risky; it requires careful design and knowledge of FFI mechanisms or multiple scripting languages. It’s work on the core and will improve the plug-in story, but is not absolutely essential.

--------

## Flush out unit tests

### Provide ability to write a more robust set of tests
We have some RPC tests but being able to express tests at a higher level (type  ‘A’, move cursor to X:Y, add cursor at A:B, transpose, paste “XYZ”, etc) would make it much easier to express the operations in a maintainable way & provide for ways to easily validate.  

Follow-on work could include "property testing," which generates test cases automatically (resulting in higher coverage) and tests properties such as correctness of incremental algorithms.

### Outcome
Tests can be easily added without a lot of effort to validate complex editing interactions. The student would evaluate the ease with which tests can be written by having other members utilize the new infrastructure to contribute a few tests. A design doc would briefly outline what a unit test would look like.

### Difficulty
This is a medium to hard difficulty focusing on testing infrastructure, not risky. Most of the difficulty is around making it easy to write and maintain unit tests as well as the infrastructure itself. For example, a DSL might be written to initialize the state of the eidtor along the lines of [ABC)b|e(DEF] which would mean that the document contains the string “ABCbeDEF”, ABC is selected from A->C, there’s a cursor between b & e, and DEF is selected F->D.

--------

## Memory map text buffers

### Improve performance editing large files
A very common function for text editors is to open large files. Xi’s RPC architecture means it has to be smarter to avoid copying the full document over IPC by sending only the visible portion of a document + small deltas as changes are made. Plugins that need full document access however (code analyzer, refactoring, search tools, etc) will end up needing to maintain a copy of the full document in-process. This means that the startup time for plugins will be really slow (as the document is copied several times to serialize to JSON, write into a file descriptor, read from a file descriptor & then deserialize again). Additionally, the memory usage will scale at O(mn) where m is the size of the document and n is the number of plugins which is undesirable.  Additionally, some front-end features like printing still require full access to the document. Memory-mapping the buffer data offers a way to share 1 copy between all processes without any copying occuring. IPC will just need to synchronize the pointers into that buffer.

### Outcome
An MVP outcome is for the core infrastructure to store the buffer data (original file + deltas) in one or two contiguous buffers. The rope data structure would then simply reference regions into that buffer. An ideal outcome would be to also provide a mechanism for mapping those buffers into plugins/frontends, building up the rope data structure in plugins/frontends & an IPC mechanism for keeping everything in-sync. A final outcome would be to come up with a good corpus of benchmarks to show the speed & memory usage impact.

### Difficulty
This would entail a high degree of difficulty and require risky changes to a fundamental data structure. It would require altering the rope data structure to be able to have a pointer to a string slice rather than own a string itself. It would require altering the file reading code to carefully fill in buffers that have been carefully allocated on page-aligned boundaries. Editing infrastructure + ropes would have to learn how to fill a different set of memory pages that could be shared. Then, there would need to be RPC work done so that all these data structures synchronized very quickly. The final piece would be swapping out the existing buffer synchronization mechanism with the mmap piece. The existing model synchronization IPC will probably remain useful for intermachine scenarios (collaborative editing, editing remote files, etc).
