---
layout: page
title: Contribute
site_nav_category: contribute
site_nav_category_order: 400
is_site_nav_category: true
---

The xi-editor project is committed to fostering and preserving a
diverse, welcoming community; all participants are expected to
follow the [Code of Conduct](https://github.com/xi-editor/xi-editor/blob/master/CODE_OF_CONDUCT.md).

- [Getting Started](#getting-started)
    - [Very first steps](#very-first-steps)
    - [Opening issues](#opening-issues)
    - [Participating in discussions](#participating-in-discussions)
    - [Improving and reviewing docs](#improving-and-reviewing-docs)
    - [Reviewing and testing changes](#reviewing-and-testing-changes)
- [Proposing and making changes](#proposing-and-making-changes)
    - [Finding something to work on](#finding-something-to-work-on)
    - [Before you start](#before-you-start-work)
    - [Before you open your PR](#before-you-open-your-pr)
    - [Review process](#review-process)
    - [After submitting your change](#after-submitting-your-change)
- [Getting more involved](#getting-more-involved)

## Getting started

### Very first steps

Not sure where to start? If you haven't already, take a look at the
[docs](http://xi-editor.github.io/xi-editor/docs.html) to get a better
sense of the project. Read through some issues and some open PRs, to
get a sense for the habits of existing contributors. Drop by the #xi
channel on [irc.mozilla.org](https://mozilla.logbot.info/xi) to follow
ongoing discussions or ask questions. Clone the repos you're
interested in, and make sure you can build and run the tests. If you
can't, open an issue, and someone will try to help. Once you're up and
running, there are a a number of ways to participate:

### Opening issues

If you have a question or a feature request or think you've found a bug,
please open an issue. When opening an issue, include any details that
might be relevant: for a bug this might be the steps required to
reproduce; for a feature request it might be a detailed explanation of
the behaviour you are imagining, an outline of how it would be used,
and/or examples of how this feature is used in other editors.

#### Before you open an issue

Before opening an issue, **try to identify where the issue belongs**.
Is it a problem with the frontend or with core? The frontend is
responsible for drawing windows and UI, and handling events; the core
is responsible for most everything else. Issues with the frontend
should be opened in that frontend's repository, and issues with
core should be opened in the
[xi-editor](https://github.com/xi-editor/xi-editor/issues) repo.

Finally, before opening an issue, **use github's search bar** to make
sure there isn't an existing (open or closed) issue for your particular
problem.

### Participating in discussions

An _explicit_ goal of xi-editor is to be an educational resource.
Everyone is encouraged to participate in discussion issues (issues with
the 'discussion' or 'planning' labels), and we expect people
participating in discussions to be respectful of the fact that we all
have different backgrounds and levels of experience. Similarly, if
something is confusing, feel free to ask for clarification! If you're
confused, other people probably are as well.

### Improving and reviewing docs

If the docs are unclear or incomplete, please open an issue or a PR to
improve them. This is an especially valuable form of feedback from new
contributors, who are seeing things for the first time, and will be best
positioned to identify areas that need improvement.

### Reviewing and testing changes

One of the best ways to get more familiar with the project is by reading
other people's pull requests. If there's something in a commit that you
don't understand, this is a great time to ask for clarification. Testing
changes is also very helpful, especially for bug fixes or feature
additions. Check out a change and try it out; does it work? Can you find
edge cases? Manual testing is very valuable. For more information on
reviews, see [code review process](#review-process).


## Proposing and making changes

### Finding something to work on

If you're looking for something to work on, a good first step is to browse
the [issues](https://github.com/xi-editor/xi-editor/issues). Specifically,
issues that are labeled
[help wanted](https://github.com/xi-editor/xi-editor/issues?q=is%3Aissue+is%3Aopen+label%3A%22help+wanted%22) and/or
[easy](https://github.com/xi-editor/xi-editor/issues?q=is%3Aissue+is%3Aopen+label%3Aeasy)
are good places to start. If you can't find anything there, feel free to ask
on IRC, or play around with the editor and try to identify something that
_you_ think is missing.

### Before you start work

Before starting to work on an issue, consider the following:

- _Is it a bugfix or small change?_ If you notice a small bug somewhere,
 and you believe you have a fix, feel free to open a pull request directly.

- _Is it a feature?_ If you have an idea for a new editor feature that is
 along the lines of something that already exists (for instance, adding a
 new command to reverse the letters in a selected region) _consider_
 opening a short issue beforehand, describing the feature you have in mind.
 Other contributors might be able to identify possible issues or
 refinements. This isn't _necessary_, but it might end up saving you work,
 and it means you will get to close an issue when your PR gets merged,
 which feels good.

- _Is it a major feature, affecting for instance the behaviour or appearance
 of a frontend, or the API or architecture of core?_ Before working on a
 large change, please open a discussion/proposal issue. This should describe
 the problem you're trying to solve, and the approach you're considering;
 think of this as a 'lite' version of Rust's
 [RFC](https://github.com/rust-lang/rfcs) process.


### Before you open your PR

Before pressing the 'Create pull request' button,

- _Run the tests_. It's easy to accidentally break something with even a small
 change, so always run the tests locally before submitting (or updating) a PR.
 You can run all checks locally with the `xi-editor/rust/run_all_checks`. script.

- _Add a message for your reviewers_. When submitting a PR, take advantage
 of the opportunity to include a message. Your goal here should be to help
 your reviewers. Are there any parts of your change that you're uncertain
 about? Are there any non-obvious explanations for some of your decisions?
 If your change changes some behaviour, how might it be tested?

- ***Be your own first reviewer***. On the page where you enter your message,
 you have a final opportunity to see your PR _as it will be seen by your
 reviewers_. This is a great opportunity to give it one last review, yourself.
 Imagine that it is someone else's work, that you're reviewing: what comments
 would you have? If you spot a typo or a problem, you can push an update in
 place, without losing your PR message or other state.

- _Add yourself to the AUTHORS file_. If this is your first substantive pull
request in this repo, feel free to add yourself to the AUTHORS file.

### Review process

Every non-trivial pull request goes through review. Everyone is welcome to
participate in review; review is an excellent time to ask questions about
code or design decisions that you don't understand.

All pull requests must be approved by an appropriate reviewer before they
are merged. For bug fixes and smaller changes, this can be anyone who has
commit rights to the repo in question; larger changes (changes which add a
feature, or significantly change behaviour or API) should also be approved by
a maintainer.

Before being merged, a change must pass
[CI](https://en.wikipedia.org/wiki/Continuous_integration).

#### Responsibilites of the approving reviewer

If you approve a change, it is expected that you:
- understand what the change is trying to do, and how it is doing it
- have manually built and tested the change, to verify it works as intended
- believe the change generally matches the idioms, formatting rules,
and overall coding style of the relevant repo
- are ready and able to help resolve any problems that may be introduced by
merging the change.

If a PR is made by a contributor who has write access to the repo in question,
they are responsible for merging/rebasing the PR after it has been approved;
otherwise it will be merged by the reviewer.

If a patch adds or modifies behaviour that is observable in the client,
the reviewer should build the patch and verify that it works as expected.

### After submitting your change

You've done all this, and submitted your patch. What now?

_Read other PRs_. If you're waiting for a review, it's likely that other
pull requests are waiting for review as well. This can be a good time
to go and take a look at what other work is happening in the project;
and if another PR has review comments, it might provide a clue to the
type of feedback you might expect.

_Patience_. As a general goal, we try to respond to all pull requests
within a few days, and to do preliminary review within a week, but we
don't always succeed. If you've opened a PR and haven't heard from
anyone, feel free to comment on it, or stop by the IRC channel, to ask
if anyone has had a chance to take a look. It's very possible that it's
been lost in the shuffle.

## Getting more involved

If you are participating in the xi-editor project, you may receive
additional privileges:

_Organization membership_: If you are regularly making contributions
to a xi project, in any of the forms outlined above, we will be happy to
add you to the xi-editor organization, which will give you the ability
to do things like add labels to issues and view active projects.

_Contributor_: If you are regularly making substantive contributions
to a specific xi project, we will be happy to add you as a contributor
to the repo in question. Contributors are encouraged to review and
approve changes, respond to issues, and generally help to maintain
the project in question.

_Maintainer_: If you are making substantive contributions to multiple
repos over an extended period, you are regularly reviewing the work of
other contributors, and you are actively participating in planning and
discussion, you may, (at the  discretion of @raphlinus) be invited to
take on the role of _maintainer_. Maintainers are responsible for
coordinating the general direction of the project, resolving
architectural questions, and doing the day to day work of moving the
project forward.
