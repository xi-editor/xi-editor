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

//! Implementation for Language Server Client

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::process;

use jsonrpc_lite::{Error, Id, JsonRpc, Params};
use serde_json::{to_value, Value};
use url::Url;
use xi_plugin_lib::CoreProxy;

use crate::lsp_types::*;
use crate::result_queue::ResultQueue;
use crate::types::Callback;
use crate::xi_core::ViewId;

/// A type to abstract communication with the language server
pub struct LanguageServerClient {
    writer: Box<dyn Write + Send>,
    pending: HashMap<u64, Callback>,
    next_id: u64,
    language_id: String,
    pub result_queue: ResultQueue,
    pub status_items: HashSet<String>,
    pub core: CoreProxy,
    pub is_initialized: bool,
    pub opened_documents: HashMap<ViewId, Url>,
    pub server_capabilities: Option<ServerCapabilities>,
    pub file_extensions: Vec<String>,
}

/// Prepare Language Server Protocol style JSON String from
/// a serde_json object `Value`
fn prepare_lsp_json(msg: &Value) -> Result<String, serde_json::error::Error> {
    let request = serde_json::to_string(&msg)?;
    Ok(format!("Content-Length: {}\r\n\r\n{}", request.len(), request))
}

/// Get numeric id from the request id.
fn number_from_id(id: &Id) -> u64 {
    match *id {
        Id::Num(n) => n as u64,
        Id::Str(ref s) => u64::from_str_radix(s, 10).expect("failed to convert string id to u64"),
        _ => panic!("unexpected value for id: None"),
    }
}

impl LanguageServerClient {
    pub fn new(
        writer: Box<dyn Write + Send>,
        core: CoreProxy,
        result_queue: ResultQueue,
        language_id: String,
        file_extensions: Vec<String>,
    ) -> Self {
        LanguageServerClient {
            writer,
            pending: HashMap::new(),
            next_id: 1,
            is_initialized: false,
            core,
            result_queue,
            status_items: HashSet::new(),
            language_id,
            server_capabilities: None,
            opened_documents: HashMap::new(),
            file_extensions,
        }
    }

    pub fn write(&mut self, msg: &str) {
        self.writer.write_all(msg.as_bytes()).expect("error writing to stdin");

        self.writer.flush().expect("error flushing child stdin");
    }

    pub fn handle_message(&mut self, message: &str) {
        match JsonRpc::parse(message) {
            Ok(JsonRpc::Request(obj)) => trace!("client received unexpected request: {:?}", obj),
            Ok(value @ JsonRpc::Notification(_)) => {
                self.handle_notification(value.get_method().unwrap(), value.get_params().unwrap())
            }
            Ok(value @ JsonRpc::Success(_)) => {
                let id = number_from_id(&value.get_id().unwrap());
                let result = value.get_result().unwrap();
                self.handle_response(id, Ok(result.clone()));
            }
            Ok(value @ JsonRpc::Error(_)) => {
                let id = number_from_id(&value.get_id().unwrap());
                let error = value.get_error().unwrap();
                self.handle_response(id, Err(error.clone()));
            }
            Err(err) => error!("Error in parsing incoming string: {}", err),
        }
    }

    pub fn handle_response(&mut self, id: u64, result: Result<Value, Error>) {
        let callback = self
            .pending
            .remove(&id)
            .unwrap_or_else(|| panic!("id {} missing from request table", id));
        callback.call(self, result);
    }

    pub fn handle_notification(&mut self, method: &str, params: Params) {
        trace!("Notification Received =>\n Method: {}, params: {:?}", method, params);
        match method {
            "window/showMessage" => {}
            "window/logMessage" => {}
            "textDocument/publishDiagnostics" => {}
            "telemetry/event" => {}
            _ => self.handle_misc_notification(method, params),
        }
    }

    pub fn handle_misc_notification(&mut self, method: &str, params: Params) {
        match self.language_id.to_lowercase().as_ref() {
            "rust" => self.handle_rust_misc_notification(method, params),
            _ => warn!("Unknown notification: {}", method),
        }
    }

    fn remove_status_item(&mut self, id: &str) {
        self.status_items.remove(id);
        for view_id in self.opened_documents.keys() {
            self.core.remove_status_item(*view_id, id);
        }
    }

    fn add_status_item(&mut self, id: &str, value: &str, alignment: &str) {
        self.status_items.insert(id.to_string());
        for view_id in self.opened_documents.keys() {
            self.core.add_status_item(*view_id, id, value, alignment);
        }
    }

    fn update_status_item(&mut self, id: &str, value: &str) {
        for view_id in self.opened_documents.keys() {
            self.core.update_status_item(*view_id, id, value);
        }
    }

    pub fn send_request(&mut self, method: &str, params: Params, completion: Callback) {
        let request = JsonRpc::request_with_params(Id::Num(self.next_id as i64), method, params);

        self.pending.insert(self.next_id, completion);
        self.next_id += 1;

        self.send_rpc(&to_value(&request).unwrap());
    }

    fn send_rpc(&mut self, value: &Value) {
        let rpc = match prepare_lsp_json(value) {
            Ok(r) => r,
            Err(err) => panic!("Encoding Error {:?}", err),
        };

        trace!("Sending RPC: {:?}", rpc);
        self.write(rpc.as_ref());
    }

    pub fn send_notification(&mut self, method: &str, params: Params) {
        let notification = JsonRpc::notification_with_params(method, params);
        let res = to_value(&notification).unwrap();
        self.send_rpc(&res);
    }
}

/// Methods to abstract sending notifications and requests to the language server
impl LanguageServerClient {
    /// Send the Initialize Request given the Root URI of the
    /// Workspace. It is None for non-workspace projects.
    pub fn send_initialize<CB>(&mut self, root_uri: Option<Url>, on_init: CB)
    where
        CB: 'static + Send + FnOnce(&mut LanguageServerClient, Result<Value, Error>),
    {
        let client_capabilities = ClientCapabilities::default();

        let init_params = InitializeParams {
            process_id: Some(u64::from(process::id())),
            root_uri,
            root_path: None,
            initialization_options: None,
            capabilities: client_capabilities,
            trace: Some(TraceOption::Verbose),
            workspace_folders: None,
        };

        let params = Params::from(serde_json::to_value(init_params).unwrap());
        self.send_request("initialize", params, Box::new(on_init));
    }

    /// Send textDocument/didOpen Notification to the Language Server
    pub fn send_did_open(&mut self, view_id: ViewId, document_uri: Url, document_text: String) {
        self.opened_documents.insert(view_id, document_uri.clone());

        let text_document_did_open_params = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                language_id: self.language_id.clone(),
                uri: document_uri,
                version: 0,
                text: document_text,
            },
        };

        let params = Params::from(serde_json::to_value(text_document_did_open_params).unwrap());
        self.send_notification("textDocument/didOpen", params);
    }

    /// Send textDocument/didClose Notification to the Language Server
    pub fn send_did_close(&mut self, view_id: ViewId) {
        let uri = self.opened_documents[&view_id].clone();
        let text_document_did_close_params =
            DidCloseTextDocumentParams { text_document: TextDocumentIdentifier { uri } };

        let params = Params::from(serde_json::to_value(text_document_did_close_params).unwrap());
        self.send_notification("textDocument/didClose", params);

        self.opened_documents.remove(&view_id);
    }

    /// Send textDocument/didChange Notification to the Language Server
    pub fn send_did_change(
        &mut self,
        view_id: ViewId,
        changes: Vec<TextDocumentContentChangeEvent>,
        version: u64,
    ) {
        let text_document_did_change_params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: self.opened_documents[&view_id].clone(),
                version: Some(version),
            },
            content_changes: changes,
        };

        let params = Params::from(serde_json::to_value(text_document_did_change_params).unwrap());
        self.send_notification("textDocument/didChange", params);
    }

    /// Send textDocument/didSave notification to the Language Server
    pub fn send_did_save(&mut self, view_id: ViewId, _document_text: &str) {
        // Add support for sending document text as well. Currently missing in LSP types
        // and is optional in LSP Specification
        let text_document_did_save_params = DidSaveTextDocumentParams {
            text_document: TextDocumentIdentifier { uri: self.opened_documents[&view_id].clone() },
        };
        let params = Params::from(serde_json::to_value(text_document_did_save_params).unwrap());
        self.send_notification("textDocument/didSave", params);
    }

    pub fn request_hover<CB>(&mut self, view_id: ViewId, position: Position, on_result: CB)
    where
        CB: 'static + Send + FnOnce(&mut LanguageServerClient, Result<Value, Error>),
    {
        let text_document_position_params = TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: self.opened_documents[&view_id].clone() },
            position,
        };

        let params = Params::from(serde_json::to_value(text_document_position_params).unwrap());
        self.send_request("textDocument/hover", params, Box::new(on_result))
    }
}

/// Helper methods to query the capabilities of the Language Server before making
/// a request. For example: we can check if the Language Server supports sending
/// incremental edits before proceeding to send one.
impl LanguageServerClient {
    /// Method to get the sync kind Supported by the Server
    pub fn get_sync_kind(&mut self) -> TextDocumentSyncKind {
        match self.server_capabilities.as_ref().and_then(|c| c.text_document_sync.as_ref()) {
            Some(&TextDocumentSyncCapability::Kind(kind)) => kind,
            _ => TextDocumentSyncKind::Full,
        }
    }
}

/// Language Specific Notification handling implementations
impl LanguageServerClient {
    pub fn handle_rust_misc_notification(&mut self, method: &str, params: Params) {
        match method {
            "window/progress" => {
                match params {
                    Params::Map(m) => {
                        let done = m.get("done").unwrap_or(&Value::Bool(false));
                        if let Value::Bool(done) = done {
                            let id: String =
                                serde_json::from_value(m.get("id").unwrap().clone()).unwrap();
                            if *done {
                                self.remove_status_item(&id);
                            } else {
                                let mut value = String::new();
                                if let Some(Value::String(s)) = &m.get("title") {
                                    value.push_str(&format!("{} ", s));
                                }

                                if let Some(Value::Number(n)) = &m.get("percentage") {
                                    value.push_str(&format!(
                                        "{} %",
                                        (n.as_f64().unwrap() * 100.00).round()
                                    ));
                                }

                                if let Some(Value::String(s)) = &m.get("message") {
                                    value.push_str(s);
                                }
                                // Add or update item
                                if self.status_items.contains(&id) {
                                    self.update_status_item(&id, &value);
                                } else {
                                    self.add_status_item(&id, &value, "left");
                                }
                            }
                        }
                    }
                    _ => warn!("Unexpected type"),
                }
            }
            _ => warn!("Unknown Notification from RLS: {} ", method),
        }
    }
}
