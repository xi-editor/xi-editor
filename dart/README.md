# Status: Experimental

What exists here is almost entirely boilerplate for the tooling and infrastructure needed to build out of Xi's UI as a set of Flutter Widgets.

# Dart, Flutter, & Fuchsia

This directory contains code for running a vanilla [Flutter][flutter] application (iOS & Android) and a [Fuchsia][fuchsia] specific set of [modules][modular]. While all the code in this directory is written in [Dart][dart], the directory structure isolates platform specific code:

* **modules**: Fuchsia specific application (module) code.
* **packages**: Platform agnostic Dart code separated into [pub packages][pub].
* **xi_flutter**: A vanilla Flutter application that uses the shared Dart packages and can be run on iOS or Android.

There is also a Fuchsia specific set of [FIDL services][fidl] defined in [../services](../services/).

## UI

UI code is defined as a set of [Flutter Widgets][widgets-intro] that can be imported and used in either Fuchsia Modules or the vanilla Flutter application.

## Core

**NOTE:** Currently works on Fuchsia only.

Xi core is a Rust binary built with a Fuchsia specific `rustc` (instructions below). A JSON protocol is used to communicate between Xi Core and the Xi UI.

# Development

**NOTE:** Your checkout of Xi needs to be part of the Fuchsia source tree for any of these instructions to work.

## Setup

Follow the instructions for setting up a fresh Fuchsia checkout.  Once you have the `jiri` tool installed and have imported the default manifest and updated return to these instructions.

There is an optional, Rust specific manifest that needs to be added with:

    jiri import rust https://fuchsia.googlesource.com/manifest

Then add/update the Rust projects to your Fuchsia tree with:

    jiri update

Don't forget to set up the [Fuchsia environment helpers][fuchsia-env] in `scripts/env.sh`:

    source scripts/env.sh

## Rust Toolchain

**NOTE:** This step is temporary.

A custom `rustc` build is required before Xi core can be built for Fuchsia. Start by building the [clang wrapper][clang-wrapper] in [magenta-rs][https://fuchsia.googlesource.com/magenta-rs/]:

    export RUST_TOOLS=${FUCHSIA_DIR}/rust/magenta-rs/tools
    cd $RUST_TOOLS
    clang++ -O --std=c++11 clang_wrapper.cc -o clang_wrapper
    ln -s clang_wrapper x86-64-unknown-fuchsia-ar
    ln -s clang_wrapper x86-64-unknown-fuchsia-cc
    ln -s clang_wrapper aarch64-unknown-fuchsia-ar

You can sanity-check the clang wrapper:

    ${RUST_TOOLS}/x86-64-unknown-fuchsia-cc
    clang-4.0: error: no input files

Then clone and build Rust per the [magenta-rs docs][magenta-rs-docs]:

    export RUST_ROOT=${FUCHSIA_DIR}/third_party/rust
    git clone https://github.com/rust-lang/rust.git $RUST_ROOT
    cd $RUST_ROOT

Create a config file:

	cat - <<EOF >$RUST_ROOT/config.toml
	# Config file for fuchsia target for building Rust.
	# See \`src/bootstrap/config.toml.example\` for other settings.

	[rust]    
	# Disable backtrace, as it requires additional lib support.
	backtrace = false

	[target.x86_64-unknown-fuchsia]
	cc = "${RUST_TOOLS}/x86-64-unknown-fuchsia-cc"

	[target.aarch64-unknown-fuchsia]
	# Path to the clang wrapper
	cc = "${RUST_TOOLS}/aarch64-unknown-fuchsia-cc"
	EOF

Configure and build rustc:

    ./configure --enable-rustbuild --target=x86_64-unknown-fuchsia
    ./x.py build --stage 1

## Verify Rust Build

Build the magenta examples:

    cd $FUCHSIA_DIR/rust/magenta-rs/

Configure cargo for the fuchsia target:

	mkdir .cargo
	cat - <<EOF >.cargo/config
	[target.x86_64-unknown-fuchsia]
	linker = "${RUST_TOOLS}/x86-64-unknown-fuchsia-cc"
	EOF

Build the magenta-rs example mx_toy:

    RUSTC=${RUST_ROOT}/build/x86_64-apple-darwin/stage1/bin/rustc cargo build --target=x86_64-unknown-fuchsia --example mx_toy

# Xi

Build the Rust binary for Xi core using the new `rustc`:

    cd ${FUCHSIA_DIR}/apps/xi
    RUSTC=${RUST_ROOT}/build/x86_64-apple-darwin/stage1/bin/rustc cargo build --target=x86_64-unknown-fuchsia

The command above will generate the binary in `target/x86_64-unknown-fuchsia/debug/xi-core`.

# Deploying

First configure your build:

    fset x86-64 --modules default,rust

Generate and build:

    fgen -m=default,rust
    fbuild
    ${FUCHSIA_DIR}/scripts/symlink-dot-packages.py --tree=//apps/xi/*

Assuming you have a working Acer setup an running `fboot` in a different terminal session, you reboot the Acer with the new build using:

    freboot

Optional: In another terminal you can tail the logs

    ${FUCHSIA_DIR}/out/build-magenta/tools/loglistener

You can run the mx_toy example from your host with:

	netruncmd : "@ exmaple_mx_toy"

You won't see any output, if you run `exmaple_mx_toy` on the device you should see some log output.

To run Xi from your host machine:

    netruncmd : "@ bootstrap device_runner --user-shell=dev_user_shell --user-shell-args=--root-module=xi_app

[flutter]: https://flutter.io/
[fuchsia]: https://fuchsia.googlesource.com/fuchsia/
[modular]: https://fuchsia.googlesource.com/modular/
[pub]: https://www.dartlang.org/tools/pub/get-started
[dart]: https://www.dartlang.org/
[fidl]: https://fuchsia.googlesource.com/fidl/
[widgets-intro]: https://flutter.io/widgets-intro/
[fuchsia-setup]: https://fuchsia.googlesource.com/fuchsia/+/HEAD/README.md
[fuchsia-env]: https://fuchsia.googlesource.com/fuchsia/+/HEAD/README.md#Setup-Build-Environment
[clang-wrapper]: https://fuchsia.googlesource.com/magenta-rs/+/HEAD/tools
[magenta-rs-docs]: https://fuchsia.googlesource.com/magenta-rs/+/HEAD/GETTING_STARTED.md
