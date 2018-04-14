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

use serde_json::Value;
use xi_rpc::RpcPeer;

use tabs::ViewId;
use config::Table;
use styles::ThemeSettings;
use plugins::rpc::ClientPluginInfo;
use plugins::Command;

/// An interface to the frontend.
pub struct Client(RpcPeer);

impl Client {
    pub fn new(peer: RpcPeer) -> Self {
        Client(peer)
    }

    pub fn update_view(&self, view_id: ViewId, update: &Value) {
        self.0.send_rpc_notification("update",
                                     &json!({
                                         "view_id": view_id,
                                         "update": update,
                                     }));
    }

    pub fn scroll_to(&self, view_id: ViewId, line: usize, col: usize) {
        self.0.send_rpc_notification("scroll_to",
                                     &json!({
                                         "view_id": view_id,
                                         "line": line,
                                         "col": col,
                                     }));
    }

    pub fn config_changed(&self, view_id: ViewId, changes: &Table) {
        self.0.send_rpc_notification("config_changed",
                                     &json!({
                                         "view_id": view_id,
                                         "changes": changes,
                                     }));
    }

    pub fn available_themes(&self, theme_names: Vec<String>) {
        self.0.send_rpc_notification("available_themes",
                                     &json!({"themes": theme_names}))
    }

    pub fn theme_changed(&self, name: &str, theme: &ThemeSettings) {
        self.0.send_rpc_notification("theme_changed",
                                     &json!({
                                         "name": name,
                                         "theme": theme,
                                     }));
    }

    /// Notify the client that a plugin ha started.
    pub fn plugin_started(&self, view_id: ViewId, plugin: &str) {
        self.0.send_rpc_notification("plugin_started",
                                     &json!({
                                         "view_id": view_id,
                                         "plugin": plugin,
                                     }));
    }

    /// Notify the client that a plugin ha stopped.
    ///
    /// `code` is not currently used.
    pub fn plugin_stopped(&self, view_id: ViewId, plugin: &str, code: i32) {
        self.0.send_rpc_notification("plugin_stopped",
                                     &json!({
                                         "view_id": view_id,
                                         "plugin": plugin,
                                         "code": code,
                                     }));
    }

    /// Notify the client of the available plugins.
    pub fn available_plugins(&self, view_id: ViewId,
                             plugins: &[ClientPluginInfo]) {
        self.0.send_rpc_notification("available_plugins",
                                     &json!({
                                         "view_id": view_id,
                                         "plugins": plugins }));
    }

    pub fn update_cmds(&self, view_id: ViewId,
                       plugin: &str, cmds: &[Command]) {
        self.0.send_rpc_notification("update_cmds",
                                     &json!({
                                         "view_id": view_id,
                                         "plugin": plugin,
                                         "cmds": cmds,
                                     }));
    }

    pub fn def_style(&self, style: &Value) {
        self.0.send_rpc_notification("def_style", &style)
    }

    pub fn alert<S: AsRef<str>>(&self, msg: S) {
        self.0.send_rpc_notification("alert", &json!({ "msg": msg.as_ref() }));
    }

    pub fn schedule_idle(&self, token: usize) {
        self.0.schedule_idle(token)
    }
}
