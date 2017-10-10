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

#[cfg(target_os = "fuchsia")]
extern crate magenta;
#[cfg(target_os = "fuchsia")]
extern crate magenta_sys;
#[cfg(target_os = "fuchsia")]
extern crate mxruntime;
#[cfg(target_os = "fuchsia")]
#[macro_use]
extern crate fidl;
#[cfg(target_os = "fuchsia")]
extern crate apps_ledger_services_public;

#[cfg(target_os = "fuchsia")]
extern crate sha2;

use serde_json::Value;

#[macro_use]
mod macros;

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
    #[cfg(target_os = "fuchsia")]
    pub mod fuchsia;
    pub mod styles;
    pub mod word_boundaries;
    pub mod index_set;
    pub mod selection;
    pub mod movement;
    pub mod syntax;
    pub mod layers;
    pub mod prefs;
}

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
use internal::prefs;
#[cfg(target_os = "fuchsia")]
use internal::fuchsia;

use tabs::{Documents, BufferContainerRef};
use rpc::{CoreNotification, CoreRequest};

#[cfg(target_os = "fuchsia")]
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

    #[cfg(target_os = "fuchsia")]
    pub fn set_ledger(&mut self, ledger: Ledger_Proxy, session_id: (u64, u32)) {
        self.tabs.setup_ledger(ledger, session_id);
    }
}

impl Handler for MainState {
    type Notification = CoreNotification;
    type Request = CoreRequest;

    fn handle_notification(&mut self, mut ctx: RpcCtx, rpc: Self::Notification) {
        self.tabs.handle_notification(rpc, &mut ctx)
    }

    fn handle_request(&mut self, mut ctx: RpcCtx, rpc: Self::Request)
                      -> Result<Value, RemoteError> {
        self.tabs.handle_request(rpc, &mut ctx)
    }

    fn idle(&mut self, _ctx: RpcCtx, _token: usize) {
        self.tabs.handle_idle();
    }
}
