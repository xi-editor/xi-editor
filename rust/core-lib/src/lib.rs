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

extern crate serde;
#[macro_use]
extern crate serde_json;
#[macro_use]
extern crate serde_derive;
extern crate time;
extern crate syntect;
extern crate toml;
extern crate notify;

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

use serde_json::Value;

pub mod rpc;

/// Internal data structures and logic.
///
/// These internals are not part of the public API (for the purpose of binding to
/// a front-end), but are exposed here, largely so they appear in documentation.
#[path=""]
pub mod internal {
    pub mod tabs;
    pub mod editor;
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
    pub mod watcher;
    pub mod line_cache_shadow;
}

pub use plugins::rpc as plugin_rpc;
pub use plugins::PluginPid;
pub use tabs::ViewIdentifier;
pub use syntax::SyntaxDefinition;
pub use config::{BufferItems as BufferConfig, Table as ConfigTable};

use internal::tabs;
use internal::editor;
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
use internal::watcher;
use internal::line_cache_shadow;
#[cfg(feature = "ledger")]
use internal::fuchsia;

use tabs::{Documents, BufferContainerRef};
use rpc::{CoreNotification, CoreRequest};

#[cfg(feature = "ledger")]
use apps_ledger_services_public::Ledger_Proxy;

extern crate xi_rope;
extern crate xi_unicode;
extern crate xi_rpc;

use xi_rpc::{RpcPeer, RpcCtx, Handler, RemoteError};

pub type MainPeer = RpcPeer;

pub struct MainState {
    tabs: Documents,
}

impl MainState {
    pub fn new() -> Self {
        MainState {
            tabs: Documents::new(),
        }
    }

    /// Returns a copy of the `BufferContainerRef`.
    ///
    /// This is exposed for testing purposes only.
    #[doc(hidden)]
    pub fn _get_buffers(&self) -> BufferContainerRef {
        self.tabs._get_buffers()
    }

    #[cfg(feature = "ledger")]
    pub fn set_ledger(&mut self, ledger: Ledger_Proxy, session_id: (u64, u32)) {
        self.tabs.setup_ledger(ledger, session_id);
    }
}

impl Handler for MainState {
    type Notification = CoreNotification;
    type Request = CoreRequest;

    fn handle_notification(&mut self, ctx: &RpcCtx, rpc: Self::Notification) {
        self.tabs.handle_notification(rpc, ctx)
    }

    fn handle_request(&mut self, ctx: &RpcCtx, rpc: Self::Request)
                      -> Result<Value, RemoteError> {
        self.tabs.handle_request(rpc, ctx)
    }

    fn idle(&mut self, _ctx: &RpcCtx, token: usize) {
        self.tabs.handle_idle(token);
    }
}
