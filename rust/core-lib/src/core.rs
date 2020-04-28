// Copyright 2018 The xi-editor Authors.
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

use std::io;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use serde_json::Value;

use xi_rpc::{Error as RpcError, Handler, ReadError, RemoteError, RpcCtx};

use crate::plugin_rpc::{PluginCommand, PluginNotification, PluginRequest};
use crate::plugins::{Plugin, PluginId};
use crate::rpc::*;
use crate::tabs::{CoreState, ViewId};

/// A reference to the main core state.
///
/// # Note
///
/// Various items of initial setup are dependent on how the client
/// is configured, so we defer instantiating state until we have that
/// information.
pub enum XiCore {
    // TODO: profile startup, and determine what things (such as theme loading)
    // we should be doing before client_init.
    Waiting,
    Running(Arc<Mutex<CoreState>>),
}

/// A weak reference to the main state. This is passed to plugin threads.
#[derive(Clone)]
pub struct WeakXiCore(Weak<Mutex<CoreState>>);

#[allow(dead_code)]
impl XiCore {
    pub fn new() -> Self {
        XiCore::Waiting
    }

    /// Returns `true` if the `client_started` has not been received.
    fn is_waiting(&self) -> bool {
        match *self {
            XiCore::Waiting => true,
            _ => false,
        }
    }

    /// Returns a guard to the core state. A convenience around `Mutex::lock`.
    ///
    /// # Panics
    ///
    /// Panics if core has not yet received the `client_started` message.
    pub fn inner(&self) -> MutexGuard<CoreState> {
        match self {
            XiCore::Running(ref inner) => inner.lock().unwrap(),
            XiCore::Waiting => panic!(
                "core does not start until client_started \
                 RPC is received"
            ),
        }
    }

    /// Returns a new reference to the core state, if core is running.
    fn weak_self(&self) -> Option<WeakXiCore> {
        match self {
            XiCore::Running(ref inner) => Some(WeakXiCore(Arc::downgrade(inner))),
            XiCore::Waiting => None,
        }
    }
}

/// Handler for messages originating with the frontend.
impl Handler for XiCore {
    type Notification = CoreNotification;
    type Request = CoreRequest;

    fn handle_notification(&mut self, ctx: &RpcCtx, rpc: Self::Notification) {
        use self::CoreNotification::*;

        // We allow tracing to be enabled before event `client_started`
        if let TracingConfig { enabled } = rpc {
            match enabled {
                true => xi_trace::enable_tracing(),
                false => xi_trace::disable_tracing(),
            }
            info!("tracing in core = {:?}", enabled);
            if self.is_waiting() {
                return;
            }
        }

        // wait for client_started before setting up inner
        if let ClientStarted { ref config_dir, ref client_extras_dir } = rpc {
            assert!(self.is_waiting(), "client_started can only be sent once");
            let state =
                CoreState::new(ctx.get_peer(), config_dir.clone(), client_extras_dir.clone());
            let state = Arc::new(Mutex::new(state));
            *self = XiCore::Running(state);
            let weak_self = self.weak_self().unwrap();
            self.inner().finish_setup(weak_self);
        }

        self.inner().client_notification(rpc);
    }

    fn handle_request(&mut self, _ctx: &RpcCtx, rpc: Self::Request) -> Result<Value, RemoteError> {
        self.inner().client_request(rpc)
    }

    fn idle(&mut self, _ctx: &RpcCtx, token: usize) {
        self.inner().handle_idle(token);
    }
}

impl WeakXiCore {
    /// Attempts to upgrade the weak reference. Essentially a wrapper
    /// for `Arc::upgrade`.
    fn upgrade(&self) -> Option<XiCore> {
        self.0.upgrade().map(XiCore::Running)
    }

    /// Called immediately after attempting to start a plugin,
    /// from the plugin's thread.
    pub fn plugin_connect(&self, plugin: Result<Plugin, io::Error>) {
        if let Some(core) = self.upgrade() {
            core.inner().plugin_connect(plugin)
        }
    }

    /// Called from a plugin runloop thread when the runloop exits.
    pub fn plugin_exit(&self, plugin: PluginId, error: Result<(), ReadError>) {
        if let Some(core) = self.upgrade() {
            core.inner().plugin_exit(plugin, error)
        }
    }

    /// Handles the result of an update sent to a plugin.
    ///
    /// All plugins must acknowledge when they are sent a new update, so that
    /// core can track which revisions are still 'live', that is can still
    /// be the base revision for a delta. Once a plugin has acknowledged a new
    /// revision, it can no longer send deltas against any older revision.
    pub fn handle_plugin_update(
        &self,
        plugin: PluginId,
        view: ViewId,
        response: Result<Value, RpcError>,
    ) {
        if let Some(core) = self.upgrade() {
            let _t = xi_trace::trace_block("WeakXiCore::plugin_update", &["core"]);
            core.inner().plugin_update(plugin, view, response);
        }
    }
}

/// Handler for messages originating from plugins.
impl Handler for WeakXiCore {
    type Notification = PluginCommand<PluginNotification>;
    type Request = PluginCommand<PluginRequest>;

    fn handle_notification(&mut self, ctx: &RpcCtx, rpc: Self::Notification) {
        let PluginCommand { view_id, plugin_id, cmd } = rpc;
        if let Some(core) = self.upgrade() {
            core.inner().plugin_notification(ctx, view_id, plugin_id, cmd)
        }
    }

    fn handle_request(&mut self, ctx: &RpcCtx, rpc: Self::Request) -> Result<Value, RemoteError> {
        let PluginCommand { view_id, plugin_id, cmd } = rpc;
        if let Some(core) = self.upgrade() {
            core.inner().plugin_request(ctx, view_id, plugin_id, cmd)
        } else {
            Err(RemoteError::custom(0, "core is missing", None))
        }
    }
}

#[cfg(test)]
/// Returns a non-functional `WeakXiRef`, needed to mock other types.
pub fn dummy_weak_core() -> WeakXiCore {
    use xi_rpc::test_utils::DummyPeer;
    use xi_rpc::Peer;
    let peer = Box::new(DummyPeer);
    let state = CoreState::new(&peer.box_clone(), None, None);
    let core = Arc::new(Mutex::new(state));
    WeakXiCore(Arc::downgrade(&core))
}
