// Copyright 2016 The xi-editor Authors.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! The main library for xi-core.

#![cfg_attr(
    feature = "cargo-clippy",
    allow(
        boxed_local,
        cast_lossless,
        collapsible_if,
        let_and_return,
        map_entry,
        match_as_ref,
        match_bool,
        needless_pass_by_value,
        new_without_default,
        new_without_default_derive,
        or_fun_call,
        ptr_arg,
        too_many_arguments,
        unreadable_literal,
        get_unwrap,
    )
)]

#[macro_use]
extern crate log;
extern crate regex;
extern crate serde;
#[macro_use]
extern crate serde_json;
#[macro_use]
extern crate serde_derive;
extern crate memchr;
#[cfg(feature = "notify")]
extern crate notify;
extern crate syntect;
extern crate time;
extern crate toml;

extern crate xi_rope;
extern crate xi_rpc;
extern crate xi_trace;
extern crate xi_trace_dump;
extern crate xi_unicode;

#[cfg(feature = "ledger")]
mod ledger_includes {
    extern crate fuchsia_zircon;
    extern crate fuchsia_zircon_sys;
    extern crate mxruntime;
    #[macro_use]
    extern crate fidl;
    extern crate apps_ledger_services_public;
    extern crate sha2;
}
#[cfg(feature = "ledger")]
use ledger_includes::*;

pub mod backspace;
pub mod client;
pub mod config;
pub mod core;
pub mod edit_types;
pub mod editor;
pub mod event_context;
pub mod file;
pub mod find;
#[cfg(feature = "ledger")]
pub mod fuchsia;
pub mod index_set;
pub mod layers;
pub mod line_cache_shadow;
pub mod line_ending;
pub mod linewrap;
pub mod movement;
pub mod plugins;
pub mod recorder;
pub mod selection;
pub mod styles;
pub mod syntax;
pub mod tabs;
pub mod view;
#[cfg(feature = "notify")]
pub mod watcher;
pub mod whitespace;
pub mod width_cache;
pub mod word_boundaries;

pub mod rpc;

#[cfg(feature = "ledger")]
use apps_ledger_services_public::Ledger_Proxy;

pub use config::{BufferItems as BufferConfig, Table as ConfigTable};
pub use core::{WeakXiCore, XiCore};
pub use editor::EditType;
pub use plugins::manifest as plugin_manifest;
pub use plugins::rpc as plugin_rpc;
pub use plugins::PluginPid;
pub use syntax::{LanguageDefinition, LanguageId};
pub use tabs::test_helpers;
pub use tabs::{BufferId, BufferIdentifier, ViewId};

// TODO
pub mod writer;

