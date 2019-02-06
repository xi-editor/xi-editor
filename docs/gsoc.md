---
layout: page
title: GSoC
site_nav_category_order: 300
is_site_nav_category: true
site_nav_category: gsoc
---

Please, use the suggested [proposal template](gsoc-template.html) when applying. Check out [GSoC guides](https://google.github.io/gsocguides/student/writing-a-proposal#elements-of-a-quality-proposal) for more tips.

Good places to start contributing to Xi are [easy](https://github.com/xi-editor/xi-editor/issues?q=is%3Aopen+is%3Aissue+label%3Aeasy) and [help wanted](https://github.com/xi-editor/xi-editor/issues?q=is%3Aopen+is%3Aissue+label%3A%22help+wanted%22) issues on GitHub.

--------

List of project ideas for Google Summer of Code 2019:
 - [Plugin to show inline compiler messages](#plugin-to-show-inline-compiler-messages)
 - [Benchmarks and tracking performance regressions](#benchmarks-and-tracking-performance-regressions)
 - [Binary size CI tool](#binary-size-ci-tool)
 - [xi-trace cleanup](#xi-trace-cleanup)

--------

## Plugin to show inline compiler messages 

Rust, Swift

@scholtzan

### Write a plugin that displays inline compiler errors and warnings
Xi editor recently introduced annotations as a way to represent additional information about regions of a document ([Annotation RFC](https://github.com/xi-editor/xi-editor/blob/master/rfcs/2018-11-23-annotations.md)). Annotations can be used to represent compiler errors, warnings and other diagnostic messages. These messages are displayed in-line of the document and are very helpful when it comes to debugging written code.

### Outcome
The end result of this project is a plugin that compiles Rust code and represents error messages and warnings as annotations. Additionally, it requires some frontend work (preferably for xi-mac in Swift) to display these annotations.
Follow-on work could include adding annotations to indicate failed tests and syntax checks/suggestions.

### Difficulty
This is a medium difficulty and pretty self-contained project. The main difficulty lies in having to write the plugin in Rust and add support to the core and the mac frontend. There already exist some sample plugins which can be used for reference.

--------

## Benchmarks and tracking performance regressions

Rust

@dsp

### Improve our current set of benchmarks, and ideally work on a tool to handle tracking of performance over time.
Performance is one of our major goals, and performance requires measurement. This project has two components: the first is to improve our current benchmark suite to cover more of the project and to include more 'macro' benchmarks (measures of overall system performance); and the second is to design a small utility that will run this benchmark suite, collect results, and store them for long term tracking and analysis of performance changes.

### Outcome
This project should produce two artifacts: a comprehensive benchmark suite, and a command line tool that runs this suite, collects the results, and stores them in a way that allows comparison over time.

### Difficulty
Flexible. Writing additional benchmarks should be a medium difficulty project, but the second part is open ended, with more and less ambitious possible designs.

--------

## Binary size CI tool

Rust, Software Engineering, GitHub API

@cmyr

### Write a github integration to report the difference in binary size between a PR and the current build.
Binary size is an important metric for us, and it is easy to overlook. It would be nice if we had some sort of github integration (a bot or action or app) that would automatically do a release build of a new PR, and report the difference in binary size between that PR and the current master branch.

### Outcome
There should be a tool available, ideally that would work for any rust project (and potentially any project at all) that would provide this functionality.

### Difficulty
Easy/Medium. Building and comparing size should be a fairly trivial problem; the difficult part will be navigating the github API and figuring out the best approach. Additionally, more ambitious goals (such as making it a universal tool) will be more challenging.

--------

## xi-trace cleanup

Rust, Documentation

@cmyr

### Clean up and release xi-trace as a standalone library.
We have a rust crate, xi-trace, that collects performance information and writes it in the [chrome trace format](https://www.chromium.org/developers/how-tos/trace-event-profiling-tool). This is an excellent tool for measuring and evaluating performance, and it is unfortunate that it is not currently used by more rust projects. Achieving this will mostly be a matter of cleaning up this library, resolving some API concerns, improving documentation, and publishing to crates.io. This is a project of numerous small parts, and will provide a good overview of the nuts and bolts of open source.

### Outcome
The xi-trace crate (potentially renamed) should be available through crates.io, with documentation that explains its use and makes it easy to get started. This release should be shared with the rust community via things like /r/rust and users.rust-lang.org.

### Difficulty
Medium
