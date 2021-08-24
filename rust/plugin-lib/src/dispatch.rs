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

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::{self, Value};

use crate::core_proxy::CoreProxy;
use crate::xi_core::plugin_rpc::{HostNotification, HostRequest, PluginBufferInfo, PluginUpdate};
use crate::xi_core::{ConfigTable, LanguageId, PluginPid, ViewId};
use xi_rpc::{Handler as RpcHandler, RemoteError, RpcCtx};
use xi_trace::{self, trace, trace_block, trace_block_payload};

use super::{Plugin, View};

/// Convenience for unwrapping a view, when handling RPC notifications.
macro_rules! bail {
    ($opt:expr, $method:expr, $pid:expr, $view:expr) => {
        match $opt {
            Some(t) => t,
            None => {
                warn!("{:?} missing {:?} for {:?}", $pid, $view, $method);
                return;
            }
        }
    };
}

/// Convenience for unwrapping a view when handling RPC requests.
/// Prints an error if the view is missing, and returns an appropriate error.
macro_rules! bail_err {
    ($opt:expr, $method:expr, $pid:expr, $view:expr) => {
        match $opt {
            Some(t) => t,
            None => {
                warn!("{:?} missing {:?} for {:?}", $pid, $view, $method);
                return Err(RemoteError::custom(404, "missing view", None));
            }
        }
    };
}

/// Handles raw RPCs from core, updating state and forwarding calls
/// to the plugin,
pub struct Dispatcher<'a, P: 'a + Plugin> {
    //TODO: when we add multi-view, this should be an Arc+Mutex/Rc+RefCell
    views: HashMap<ViewId, View<P::Cache>>,
    pid: Option<PluginPid>,
    plugin: &'a mut P,
}

impl<'a, P: 'a + Plugin> Dispatcher<'a, P> {
    pub(crate) fn new(plugin: &'a mut P) -> Self {
        Dispatcher { views: HashMap::new(), pid: None, plugin }
    }

    fn do_initialize(
        &mut self,
        ctx: &RpcCtx,
        plugin_id: PluginPid,
        buffers: Vec<PluginBufferInfo>,
    ) {
        assert!(self.pid.is_none(), "initialize rpc received with existing pid");
        info!("Initializing plugin {:?}", plugin_id);
        self.pid = Some(plugin_id);

        let core_proxy = CoreProxy::new(self.pid.unwrap(), ctx);
        self.plugin.initialize(core_proxy);

        self.do_new_buffer(ctx, buffers);
    }

    fn do_did_save(&mut self, view_id: ViewId, path: PathBuf) {
        let v = bail!(self.views.get_mut(&view_id), "did_save", self.pid, view_id);
        let prev_path = v.path.take();
        v.path = Some(path);
        self.plugin.did_save(v, prev_path.as_deref());
    }

    fn do_config_changed(&mut self, view_id: ViewId, changes: &ConfigTable) {
        let v = bail!(self.views.get_mut(&view_id), "config_changed", self.pid, view_id);
        self.plugin.config_changed(v, changes);
        for (key, value) in changes.iter() {
            v.config_table.insert(key.to_owned(), value.to_owned());
        }
        let conf = serde_json::from_value(Value::Object(v.config_table.clone()));
        v.config = conf.unwrap();
    }

    fn do_language_changed(&mut self, view_id: ViewId, new_lang: LanguageId) {
        let v = bail!(self.views.get_mut(&view_id), "language_changed", self.pid, view_id);
        let old_lang = v.language_id.clone();
        v.set_language(new_lang);
        self.plugin.language_changed(v, old_lang);
    }

    fn do_custom_command(&mut self, view_id: ViewId, method: &str, params: Value) {
        let v = bail!(self.views.get_mut(&view_id), method, self.pid, view_id);
        self.plugin.custom_command(v, method, params);
    }

    fn do_new_buffer(&mut self, ctx: &RpcCtx, buffers: Vec<PluginBufferInfo>) {
        let plugin_id = self.pid.unwrap();
        buffers
            .into_iter()
            .map(|info| View::new(ctx.get_peer().clone(), plugin_id, info))
            .for_each(|view| {
                let mut view = view;
                self.plugin.new_view(&mut view);
                self.views.insert(view.view_id, view);
            });
    }

    fn do_close(&mut self, view_id: ViewId) {
        {
            let v = bail!(self.views.get(&view_id), "close", self.pid, view_id);
            self.plugin.did_close(v);
        }
        self.views.remove(&view_id);
    }

    fn do_shutdown(&mut self) {
        info!("rust plugin lib does not shutdown");
        //TODO: handle shutdown
    }

    fn do_get_hover(&mut self, view_id: ViewId, request_id: usize, position: usize) {
        let v = bail!(self.views.get_mut(&view_id), "get_hover", self.pid, view_id);
        self.plugin.get_hover(v, request_id, position)
    }

    fn do_tracing_config(&mut self, enabled: bool) {
        if enabled {
            xi_trace::enable_tracing();
            info!("Enabling tracing in global plugin {:?}", self.pid);
            trace("enable tracing", &["plugin"]);
        } else {
            xi_trace::disable_tracing();
            info!("Disabling tracing in global plugin {:?}", self.pid);
            trace("disable tracing", &["plugin"]);
        }
    }

    fn do_update(&mut self, update: PluginUpdate) -> Result<Value, RemoteError> {
        let _t = trace_block("Dispatcher::do_update", &["plugin"]);
        let PluginUpdate {
            view_id,
            delta,
            new_len,
            new_line_count,
            rev,
            undo_group,
            edit_type,
            author,
        } = update;
        let v = bail_err!(self.views.get_mut(&view_id), "update", self.pid, view_id);
        v.update(delta.as_ref(), new_len, new_line_count, rev, undo_group);
        self.plugin.update(v, delta.as_ref(), edit_type, author);

        Ok(Value::from(1))
    }

    fn do_collect_trace(&self) -> Result<Value, RemoteError> {
        use xi_trace::chrome_trace_dump;

        let samples = xi_trace::samples_cloned_unsorted();
        chrome_trace_dump::to_value(&samples).map_err(|e| RemoteError::Custom {
            code: 0,
            message: format!("Could not serialize trace: {:?}", e),
            data: None,
        })
    }
}

impl<'a, P: Plugin> RpcHandler for Dispatcher<'a, P> {
    type Notification = HostNotification;
    type Request = HostRequest;

    fn handle_notification(&mut self, ctx: &RpcCtx, rpc: Self::Notification) {
        use self::HostNotification::*;
        let _t = trace_block("Dispatcher::handle_notif", &["plugin"]);
        match rpc {
            Initialize { plugin_id, buffer_info } => {
                self.do_initialize(ctx, plugin_id, buffer_info)
            }
            DidSave { view_id, path } => self.do_did_save(view_id, path),
            ConfigChanged { view_id, changes } => self.do_config_changed(view_id, &changes),
            NewBuffer { buffer_info } => self.do_new_buffer(ctx, buffer_info),
            DidClose { view_id } => self.do_close(view_id),
            Shutdown(..) => self.do_shutdown(),
            TracingConfig { enabled } => self.do_tracing_config(enabled),
            GetHover { view_id, request_id, position } => {
                self.do_get_hover(view_id, request_id, position)
            }
            LanguageChanged { view_id, new_lang } => self.do_language_changed(view_id, new_lang),
            CustomCommand { view_id, method, params } => {
                self.do_custom_command(view_id, &method, params)
            }
            Ping(..) => (),
        }
    }

    fn handle_request(&mut self, _ctx: &RpcCtx, rpc: Self::Request) -> Result<Value, RemoteError> {
        use self::HostRequest::*;
        let _t = trace_block("Dispatcher::handle_request", &["plugin"]);
        match rpc {
            Update(params) => self.do_update(params),
            CollectTrace(..) => self.do_collect_trace(),
        }
    }

    fn idle(&mut self, _ctx: &RpcCtx, token: usize) {
        let _t = trace_block_payload("Dispatcher::idle", &["plugin"], format!("token: {}", token));
        let view_id: ViewId = token.into();
        let v = bail!(self.views.get_mut(&view_id), "idle", self.pid, view_id);
        self.plugin.idle(v);
    }
}
