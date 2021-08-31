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

//! Requests and notifications from the core to front-ends.

use std::time::Instant;

use serde_json::{self, Value};
use xi_rpc::{self, RpcPeer};

use crate::config::Table;
use crate::plugins::rpc::ClientPluginInfo;
use crate::plugins::Command;
use crate::styles::ThemeSettings;
use crate::syntax::LanguageId;
use crate::tabs::ViewId;
use crate::width_cache::{WidthReq, WidthResponse};

/// An interface to the frontend.
pub struct Client(RpcPeer);

impl Client {
    pub fn new(peer: RpcPeer) -> Self {
        Client(peer)
    }

    pub fn update_view(&self, view_id: ViewId, update: &Update) {
        self.0.send_rpc_notification(
            "update",
            &json!({
                "view_id": view_id,
                "update": update,
            }),
        );
    }

    pub fn scroll_to(&self, view_id: ViewId, line: usize, col: usize) {
        self.0.send_rpc_notification(
            "scroll_to",
            &json!({
                "view_id": view_id,
                "line": line,
                "col": col,
            }),
        );
    }

    pub fn config_changed(&self, view_id: ViewId, changes: &Table) {
        self.0.send_rpc_notification(
            "config_changed",
            &json!({
                "view_id": view_id,
                "changes": changes,
            }),
        );
    }

    pub fn available_themes(&self, theme_names: Vec<String>) {
        self.0.send_rpc_notification("available_themes", &json!({ "themes": theme_names }))
    }

    pub fn available_languages(&self, languages: Vec<LanguageId>) {
        self.0.send_rpc_notification("available_languages", &json!({ "languages": languages }))
    }

    pub fn theme_changed(&self, name: &str, theme: &ThemeSettings) {
        self.0.send_rpc_notification(
            "theme_changed",
            &json!({
                "name": name,
                "theme": theme,
            }),
        );
    }

    pub fn language_changed(&self, view_id: ViewId, new_lang: &LanguageId) {
        self.0.send_rpc_notification(
            "language_changed",
            &json!({
                "view_id": view_id,
                "language_id": new_lang,
            }),
        );
    }

    /// Notify the client that a plugin has started.
    pub fn plugin_started(&self, view_id: ViewId, plugin: &str) {
        self.0.send_rpc_notification(
            "plugin_started",
            &json!({
                "view_id": view_id,
                "plugin": plugin,
            }),
        );
    }

    /// Notify the client that a plugin has stopped.
    ///
    /// `code` is not currently used; in the future may be used to
    /// pass an exit code.
    pub fn plugin_stopped(&self, view_id: ViewId, plugin: &str, code: i32) {
        self.0.send_rpc_notification(
            "plugin_stopped",
            &json!({
                "view_id": view_id,
                "plugin": plugin,
                "code": code,
            }),
        );
    }

    /// Notify the client of the available plugins.
    pub fn available_plugins(&self, view_id: ViewId, plugins: &[ClientPluginInfo]) {
        self.0.send_rpc_notification(
            "available_plugins",
            &json!({
                "view_id": view_id,
                "plugins": plugins }),
        );
    }

    pub fn update_cmds(&self, view_id: ViewId, plugin: &str, cmds: &[Command]) {
        self.0.send_rpc_notification(
            "update_cmds",
            &json!({
                "view_id": view_id,
                "plugin": plugin,
                "cmds": cmds,
            }),
        );
    }

    pub fn def_style(&self, style: &Value) {
        self.0.send_rpc_notification("def_style", style)
    }

    pub fn find_status(&self, view_id: ViewId, queries: &Value) {
        self.0.send_rpc_notification(
            "find_status",
            &json!({
                "view_id": view_id,
                "queries": queries,
            }),
        );
    }

    pub fn replace_status(&self, view_id: ViewId, replace: &Value) {
        self.0.send_rpc_notification(
            "replace_status",
            &json!({
                "view_id": view_id,
                "status": replace,
            }),
        );
    }

    /// Ask front-end to measure widths of strings.
    pub fn measure_width(&self, reqs: &[WidthReq]) -> Result<WidthResponse, xi_rpc::Error> {
        let req_json = serde_json::to_value(reqs).expect("failed to serialize width req");
        let resp = self.0.send_rpc_request("measure_width", &req_json)?;
        Ok(serde_json::from_value(resp).expect("failed to deserialize width response"))
    }

    pub fn alert<S: AsRef<str>>(&self, msg: S) {
        self.0.send_rpc_notification("alert", &json!({ "msg": msg.as_ref() }));
    }

    pub fn add_status_item(
        &self,
        view_id: ViewId,
        source: &str,
        key: &str,
        value: &str,
        alignment: &str,
    ) {
        self.0.send_rpc_notification(
            "add_status_item",
            &json!({
                "view_id": view_id,
                "source": source,
                "key": key,
                "value": value,
                "alignment": alignment
            }),
        );
    }

    pub fn update_status_item(&self, view_id: ViewId, key: &str, value: &str) {
        self.0.send_rpc_notification(
            "update_status_item",
            &json!({
                "view_id": view_id,
                "key": key,
                "value": value,
            }),
        );
    }

    pub fn remove_status_item(&self, view_id: ViewId, key: &str) {
        self.0.send_rpc_notification(
            "remove_status_item",
            &json!({
                "view_id": view_id,
                "key": key,
            }),
        );
    }

    pub fn show_hover(&self, view_id: ViewId, request_id: usize, result: String) {
        self.0.send_rpc_notification(
            "show_hover",
            &json!({
                "view_id": view_id,
                "request_id": request_id,
                "result": result,
            }),
        )
    }

    pub fn schedule_idle(&self, token: usize) {
        self.0.schedule_idle(token)
    }

    pub fn schedule_timer(&self, timeout: Instant, token: usize) {
        self.0.schedule_timer(timeout, token);
    }
}

#[derive(Debug, Serialize)]
pub struct Update {
    pub(crate) ops: Vec<UpdateOp>,
    pub(crate) pristine: bool,
    pub(crate) annotations: Vec<Value>,
}

#[derive(Debug, Serialize)]
pub(crate) struct UpdateOp {
    op: OpType,
    n: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    lines: Option<Vec<Value>>,
    #[serde(rename = "ln")]
    #[serde(skip_serializing_if = "Option::is_none")]
    first_line_number: Option<usize>,
}

impl UpdateOp {
    pub(crate) fn invalidate(n: usize) -> Self {
        UpdateOp { op: OpType::Invalidate, n, lines: None, first_line_number: None }
    }

    pub(crate) fn skip(n: usize) -> Self {
        UpdateOp { op: OpType::Skip, n, lines: None, first_line_number: None }
    }

    pub(crate) fn copy(n: usize, line: usize) -> Self {
        UpdateOp { op: OpType::Copy, n, lines: None, first_line_number: Some(line) }
    }

    pub(crate) fn insert(lines: Vec<Value>) -> Self {
        UpdateOp { op: OpType::Insert, n: lines.len(), lines: Some(lines), first_line_number: None }
    }

    pub(crate) fn update(lines: Vec<Value>, line_opt: Option<usize>) -> Self {
        UpdateOp {
            op: OpType::Update,
            n: lines.len(),
            lines: Some(lines),
            first_line_number: line_opt,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
enum OpType {
    #[serde(rename = "ins")]
    Insert,
    Skip,
    Invalidate,
    Copy,
    Update,
}
