// Copyright 2018 Google Inc. All rights reserved.
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

use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::io;

use serde_json::Value;

use xi_rpc::{RpcCtx, Handler, RemoteError, Error as RpcError};
use xi_trace;

use plugin_rpc::{PluginCommand, PluginNotification, PluginRequest};
use plugins::{Plugin, PluginId};
use rpc::*;
use tabs::{CoreState, ViewId};


/// The main state of the xi core, protected by a mutex.
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

    fn is_waiting(&self) -> bool {
        match *self {
            XiCore::Waiting => true,
            _ => false,
        }
    }

    pub fn inner(&self) -> MutexGuard<CoreState> {
        match self {
            &XiCore::Running(ref inner) => inner.lock().unwrap(),
            &XiCore::Waiting => panic!("core does not start until client_started \
                                      RPC is received"),
        }
    }

    fn weak_self(&self) -> Option<WeakXiCore> {
        match self {
            &XiCore::Running(ref inner) =>
                Some(WeakXiCore(Arc::downgrade(inner))),
            &XiCore::Waiting => None,
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
        if let &TracingConfig { enabled } = &rpc {
            match enabled {
                true => xi_trace::enable_tracing(),
                false => xi_trace::disable_tracing(),
            }
            eprintln!("tracing in core = {:?}", enabled);
            return;
        }

        // wait for client_started before setting up inner
        if let &ClientStarted { ref config_dir, ref client_extras_dir } = &rpc {
            assert!(self.is_waiting(), "client_started can only be sent once");
            let state = CoreState::new(ctx.get_peer());
            let state = Arc::new(Mutex::new(state));
            *self = XiCore::Running(state);
            let weak_self = self.weak_self().unwrap();
            self.inner().finish_setup(weak_self, config_dir.clone(),
                                      client_extras_dir.clone());
        }

        self.inner().client_notification(rpc);

    }

    fn handle_request(&mut self, _ctx: &RpcCtx, rpc: Self::Request)
                      -> Result<Value, RemoteError> {
        self.inner().client_request(rpc)
    }

    fn idle(&mut self, _ctx: &RpcCtx, token: usize) {
        self.inner().handle_idle(token);
    }
}

impl WeakXiCore {
    fn upgrade(&self) -> Option<XiCore> {
        self.0.upgrade().map(|state| XiCore::Running(state))
    }

    pub fn plugin_connect(&self, plugin: Result<Plugin, io::Error>) {
        if let Some(core) = self.upgrade() {
            core.inner().plugin_connect(plugin)
        }
    }

    pub fn handle_plugin_update(&self, plugin: PluginId, view: ViewId,
                                undo_group: usize,
                                response: Result<Value, RpcError>) {
        if let Some(core) = self.upgrade() {
            core.inner().plugin_update(plugin, view, undo_group, response);
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

    fn handle_request(&mut self, ctx: &RpcCtx, rpc: Self::Request)
                      -> Result<Value, RemoteError> {
        let PluginCommand { view_id, plugin_id, cmd } = rpc;
        if let Some(core) = self.upgrade() {
            core.inner().plugin_request(ctx, view_id, plugin_id, cmd)
        } else {
            Err(RemoteError::custom(0, "core is missing", None))
        }
    }
}

#[cfg(test)]
pub fn dummy_weak_core() -> WeakXiCore {
    use xi_rpc::test_utils::DummyPeer;
    use xi_rpc::Peer;
    let peer = Box::new(DummyPeer);
    let state = CoreState::new(&peer.box_clone());
    let core = Arc::new(Mutex::new(state));
    WeakXiCore(Arc::downgrade(&core))
}
