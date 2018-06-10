// Copyright 2016 Google Inc. All rights reserved.
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

#![cfg_attr(feature = "cargo-clippy", allow(
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
))]

extern crate regex;
extern crate serde;
#[macro_use]
extern crate serde_json;
#[macro_use]
extern crate serde_derive;
extern crate time;
extern crate syntect;
extern crate toml;
#[cfg(feature = "notify")]
extern crate notify;

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

/// Internal data structures and logic.
///
/// These internals are not part of the public API (for the purpose of binding to
/// a front-end), but are exposed here, largely so they appear in documentation.
#[path=""]
pub mod internal {
    pub mod client;
    pub mod core;
    pub mod tabs;
    pub mod editor;
    pub mod edit_types;
    pub mod event_context;
    pub mod file;
    pub mod find;
    pub mod view;
    pub mod linewrap;
    pub mod plugins;
    #[cfg(feature = "ledger")]
    pub mod fuchsia;
    pub mod styles;
    pub mod word_boundaries;
    pub mod index_set;
    pub mod selection;
    pub mod movement;
    pub mod syntax;
    pub mod layers;
    pub mod config;
    #[cfg(feature = "notify")]
    pub mod watcher;
    pub mod line_cache_shadow;
    pub mod width_cache;
}

pub mod rpc;

use internal::tabs;
use internal::core;
use internal::client;
use internal::edit_types;
use internal::editor;
use internal::event_context;
use internal::file;
use internal::find;
use internal::view;
use internal::linewrap;
use internal::plugins;
use internal::styles;
use internal::word_boundaries;
use internal::index_set;
use internal::selection;
use internal::movement;
use internal::syntax;
use internal::layers;
use internal::config;
#[cfg(feature = "notify")]
use internal::watcher;
use internal::line_cache_shadow;
use internal::width_cache;
#[cfg(feature = "ledger")]
use internal::fuchsia;

#[cfg(feature = "ledger")]
use apps_ledger_services_public::Ledger_Proxy;

pub use config::{BufferItems as BufferConfig, Table as ConfigTable};
pub use core::{XiCore, WeakXiCore};
pub use plugins::rpc as plugin_rpc;
pub use plugins::manifest as plugin_manifest;
pub use plugins::PluginPid;
pub use syntax::{LanguageDefinition, LanguageId};
pub use tabs::{BufferId, BufferIdentifier, ViewId, ViewIdentifier};
pub use tabs::test_helpers as test_helpers;

