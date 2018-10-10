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

#[macro_use]
extern crate log;
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
extern crate memchr;

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
pub mod whitespace;
pub mod line_ending;
pub mod backspace;

pub mod rpc;

#[cfg(feature = "ledger")]
use apps_ledger_services_public::Ledger_Proxy;

pub use config::{BufferItems as BufferConfig, Table as ConfigTable};
pub use core::{XiCore, WeakXiCore};
pub use editor::EditType;
pub use plugins::rpc as plugin_rpc;
pub use plugins::manifest as plugin_manifest;
pub use plugins::PluginPid;
pub use syntax::{LanguageDefinition, LanguageId};
pub use tabs::{BufferId, BufferIdentifier, ViewId};
pub use tabs::test_helpers as test_helpers;

