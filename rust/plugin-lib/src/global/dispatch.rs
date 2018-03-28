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

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::{self, Value};

use xi_core::{ViewIdentifier, PluginPid, ConfigTable};
use xi_core::plugin_rpc::{PluginBufferInfo, PluginUpdate, HostRequest, HostNotification};
use xi_rpc::{RpcCtx, RemoteError, Handler as RpcHandler};
use xi_trace::{self, trace, trace_block, trace_block_payload};

use global::{Plugin, View};

/// Convenience for unwrapping a view, when handling RPC notifications.
macro_rules! bail {
    ($opt:expr, $method:expr, $pid:expr, $view:expr) => ( match $opt {
        Some(t) => t,
        None => {
            eprintln!("{:?} missing {:?} for {:?}", $pid, $view, $method);
            return
        }
    })
}

/// Convenience for unwrapping a view when handling RPC requests.
/// Prints an error if the view is missing, and returns an appropriate error.
macro_rules! bail_err {
    ($opt:expr, $method:expr, $pid:expr, $view:expr) => ( match $opt {
        Some(t) => t,
        None => {
            eprintln!("{:?} missing {:?} for {:?}", $pid, $view, $method);
            return Err(RemoteError::custom(404, "missing view", None))
        }
    })
}

/// Handles raw RPCs from core, updating state and forwarding calls
/// to the plugin,
pub struct Dispatcher<'a, P: 'a + Plugin> {
    //TODO: when we add multi-view, this should be an Arc+Mutex/Rc+RefCell
    views: HashMap<ViewIdentifier, View<P::Cache>>,
    pid: Option<PluginPid>,
    plugin: &'a mut P,
}

impl<'a, P: 'a + Plugin> Dispatcher<'a, P> {
    pub (crate) fn new(plugin: &'a mut P) -> Self {
        Dispatcher {
            views: HashMap::new(),
            pid: None,
            plugin: plugin,
        }
    }

    fn do_initialize(&mut self, ctx: &RpcCtx,
                     plugin_id: PluginPid,
                     buffers: Vec<PluginBufferInfo>)
    {
        assert!(self.pid.is_none(), "initialize rpc received with existing pid");
        self.pid = Some(plugin_id);
        self.do_new_buffer(ctx, buffers);
    }

    fn do_did_save(&mut self, view_id: ViewIdentifier, path: PathBuf) {
        let v = bail!(self.views.get_mut(&view_id), "did_save", self.pid, view_id);
        let prev_path = v.path.take();
        v.path = Some(path);
        self.plugin.did_save(v, prev_path.as_ref().map(PathBuf::as_path));
    }

    fn do_config_changed(&mut self, view_id: ViewIdentifier, changes: ConfigTable) {
        let v = bail!(self.views.get_mut(&view_id), "config_changed", self.pid, view_id);
        self.plugin.config_changed(v, &changes);
        for (key, value) in changes.iter() {
            v.config_table.insert(key.to_owned(), value.to_owned());
        }
        let conf = serde_json::from_value(Value::Object(v.config_table.clone()));
        v.config = conf.unwrap();
    }

    fn do_new_buffer(&mut self, ctx: &RpcCtx, buffers: Vec<PluginBufferInfo>) {
        let plugin_id = self.pid.unwrap();
        buffers.into_iter()
            .map(|info| View::new(ctx.get_peer().clone(), plugin_id, info))
            .for_each(|view| {
                let mut view = view;
                self.plugin.new_view(&mut view);
                self.views.insert(view.view_id, view);
            });

    }

    fn do_close(&mut self, view_id: ViewIdentifier) {
        {
            let v = bail!(self.views.get(&view_id), "close", self.pid, view_id);
            self.plugin.did_close(v);
        }
        self.views.remove(&view_id);
    }

    fn do_shutdown(&mut self) {
        eprintln!("rust plugin lib does not shutdown");
        //TODO: handle shutdown

    }

    fn do_tracing_config(&mut self, enabled: bool) {
        use xi_trace;

        if enabled {
            xi_trace::enable_tracing();
            eprintln!("Enabling tracing in {:?}", self.pid);
            trace("enable tracing", &["plugin"]);
        } else {
            xi_trace::disable_tracing();
            eprintln!("Disabling tracing in {:?}",  self.pid);
            trace("enable tracing", &["plugin"]);
        }
    }

    fn do_update(&mut self, update: PluginUpdate) -> Result<Value, RemoteError> {
        let _t = trace_block("Dispatcher::do_update", &["plugin"]);
        let PluginUpdate {
            view_id, delta, new_len, new_line_count, rev, edit_type, author,
        } = update;
        let v = bail_err!(self.views.get_mut(&view_id), "update",
                          self.pid, view_id);
        v.update(delta.as_ref(), new_len, new_line_count, rev);
        Ok(self.plugin.update(v, delta.as_ref(), edit_type, author)
            .map(|edit| serde_json::to_value(edit).unwrap())
            .unwrap_or(Value::from(1)))
    }

    fn do_collect_trace(&self) -> Result<Value, RemoteError> {
        use xi_trace_dump::*;

        let samples = xi_trace::samples_cloned_unsorted();
        let mut out = Vec::new();
        chrome_trace::serialize(samples.iter(),
                                chrome_trace::OutputFormat::JsonArray,
                                &mut out).unwrap();
        let traces = serde_json::from_reader(out.as_slice());
        Ok(traces?)
    }
}

impl<'a, P: Plugin> RpcHandler for Dispatcher<'a, P> {
    type Notification = HostNotification;
    type Request = HostRequest;

    fn handle_notification(&mut self, ctx: &RpcCtx, rpc: Self::Notification) {
        use self::HostNotification::*;
        let _t = trace_block("Dispatcher::handle_notif", &["plugin"]);
        match rpc {
            Initialize { plugin_id, buffer_info } =>
                self.do_initialize(ctx, plugin_id, buffer_info),
            DidSave { view_id, path } =>
                self.do_did_save(view_id, path),
            ConfigChanged { view_id, changes } =>
                self.do_config_changed(view_id, changes),
            NewBuffer { buffer_info } =>
                self.do_new_buffer(ctx, buffer_info),
            DidClose { view_id } =>
                self.do_close(view_id),
            Shutdown ( .. ) =>
                self.do_shutdown(),
            TracingConfig { enabled } =>
                self.do_tracing_config(enabled),
            Ping ( .. ) => (),
        }
    }

    fn handle_request(&mut self, _ctx: &RpcCtx, rpc: Self::Request)
                      -> Result<Value, RemoteError> {
        use self::HostRequest::*;
        let _t = trace_block("Dispatcher::handle_request", &["plugin"]);
        match rpc {
            Update(params) =>
                self.do_update(params),
            CollectTrace ( .. ) =>
                self.do_collect_trace(),
        }
    }

    fn idle(&mut self, _ctx: &RpcCtx, token: usize) {
        let _t = trace_block_payload("Dispatcher::idle", &["plugin"],
                                     format!("token: {}", token));
        let view_id: ViewIdentifier = token.into();
        let v = bail!(self.views.get_mut(&view_id), "idle", self.pid, view_id);
        self.plugin.idle(v);
    }
}


