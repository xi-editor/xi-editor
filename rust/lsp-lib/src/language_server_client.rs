// Copyright 2018 Google LLC
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

use jsonrpc_lite::{Error, Id, JsonRpc, Params};
use lsp_types::*;
use serde_json;
use serde_json::{to_value, Value};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process;
use types::Callback;
use url::Url;
use xi_core::ViewIdentifier;

pub type DoucmentURI = Url;

/// A type to abstract communication with the language server
pub struct LanguageServerClient {
    writer: Box<Write + Send>,
    pending: HashMap<usize, Callback>,
    next_id: usize,
    workspace_identifier: Option<String>,
    language_id: String,
    pub is_initialized: bool,
    pub opened_documents: HashMap<ViewIdentifier, DoucmentURI>,
    pub server_capabilities: Option<ServerCapabilities>,
    pub file_extensions: Vec<String>,
}


/// Prepare Language Server Protocol style JSON String from 
/// a serde_json object `Value`
fn prepare_lsp_json(msg: &Value) -> Result<String, serde_json::error::Error> {
    let request = serde_json::to_string(&msg)?;
    Ok(format!(
        "Content-Length: {}\r\n\r\n{}",
        request.len(),
        request
    ))
}

/// Get numeric id from the request id. 
/// TODO: Fix this hacky implementation
fn number_from_id(id: Option<&Id>) -> usize {
    
    let id = id.expect("response missing id field");
    let id = match id {
        &Id::Num(n) => n as u64,
        &Id::Str(ref s) => u64::from_str_radix(s, 10).expect("failed to convert string id to u64"),
        other => panic!("unexpected value for id field: {:?}", other),
    };

    id as usize
}

impl LanguageServerClient {
    pub fn new(
        writer: Box<Write + Send>,
        language_id: String,
        file_extensions: Vec<String>,
        workspace_identifier: Option<String>,
    ) -> Self {
        LanguageServerClient {
            writer,
            pending: HashMap::new(),
            next_id: 1,
            is_initialized: false,
            language_id,
            server_capabilities: None,
            opened_documents: HashMap::new(),
            workspace_identifier,
            file_extensions,
        }
    }

    pub fn write(&mut self, msg: &str) {
        self.writer
            .write_all(msg.as_bytes())
            .expect("error writing to stdin");

        self.writer.flush().expect("error flushing child stdin");
    }

    pub fn handle_message(&mut self, message: &str) {
        let value = JsonRpc::parse(message).unwrap();

        match value {
            JsonRpc::Request(obj) => eprintln!("client received unexpected request: {:?}", obj),
            JsonRpc::Notification(obj) => eprintln!("\n\n recv notification: {:?} \n\n", obj),
            JsonRpc::Success(_) => {
                let mut result = value.get_result().unwrap().to_owned();
                let id = number_from_id(value.get_id().as_ref());
                self.handle_response(id, result);
            }
            JsonRpc::Error(_) => {
                let mut error = value.get_error().unwrap().to_owned();
                let id = number_from_id(value.get_id().as_ref());
                self.handle_error(id, error);
            }
        };
    }

    pub fn handle_response(&mut self, id: usize, result: Value) {
        let callback = self
            .pending
            .remove(&id)
            .expect(&format!("id {} missing from request table", id));
        callback.call(self, Ok(result));
    }

    pub fn handle_error(&mut self, id: usize, error: Error) {
        let callback = self
            .pending
            .remove(&id)
            .expect(&format!("id {} missing from request table", id));
        callback.call(self, Err(error));
    }

    pub fn send_request(&mut self, method: &str, params: Params, completion: Callback) {
        let request = JsonRpc::request_with_params(Id::Num(self.next_id as i64), method, params);

        self.pending.insert(self.next_id, completion);
        self.next_id += 1;

        self.send_rpc(to_value(&request).unwrap());
    }

    fn send_rpc(&mut self, value: Value) {
        let rpc = match prepare_lsp_json(&value) {
            Ok(r) => r,
            Err(err) => panic!("Encoding Error {:?}", err),
        };

        eprintln!("RPC: {:?}", rpc);
        self.write(rpc.as_ref());
    }

    pub fn send_notification(&mut self, method: &str, params: Params) {
        let notification = JsonRpc::notification_with_params(method, params);

        let res = to_value(&notification).unwrap();
        eprintln!("RESULT: {:?}", res);

        self.send_rpc(res);
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
            process_id: Some(process::id() as u64),
            root_uri: root_uri,
            root_path: None,
            initialization_options: None,
            capabilities: client_capabilities,
            trace: None,
        };

        let params = Params::from(serde_json::to_value(init_params).unwrap());
        self.send_request("initialize", params, Box::new(on_init));
    }

    /// Send textDocument/didOpen Notification to the Language Server 
    pub fn send_did_open(
        &mut self,
        view_id: ViewIdentifier,
        document_uri: Url,
        document_text: String,
    ) {

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

    /// Send textDocument/didChange Notification to the Language Server
    pub fn send_did_change(
        &mut self,
        view_id: ViewIdentifier,
        changes: Vec<TextDocumentContentChangeEvent>,
        rev: u64,
    ) {

        let text_document_did_change_params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: self.opened_documents.get(&view_id).unwrap().clone(),
                version: Some(rev),
            },
            content_changes: changes,
        };

        eprintln!(
            "\n\n params did_change_notif :\n {:?}\n\n",
            text_document_did_change_params
        );

        let params = Params::from(serde_json::to_value(text_document_did_change_params).unwrap());
        self.send_notification("textDocument/didChange", params);
    }

    /// Send textDocument/didSave notification to the Language Server
    pub fn send_did_save(&mut self, view_id: ViewIdentifier, _document_text: String) {
        // Add support for sending document text as well. Currently missing in LSP types
        // and is optional in LSP Specification
        let text_document_did_save_params = DidSaveTextDocumentParams {
            text_document: TextDocumentIdentifier {
                uri: self.opened_documents.get(&view_id).unwrap().clone(),
            },
        };

        let params = Params::from(serde_json::to_value(text_document_did_save_params).unwrap());
        self.send_notification("textDocument/didSave", params);
    }
}

/// Helper methods to query the capabilities of the Language Server before making
/// a request. For example: we can check if the Language Server supports sending
/// incremental edits before proceeding to send one.
impl LanguageServerClient {
    
    /// Method to get the sync kind Supported by the Server
    pub fn get_sync_kind(&mut self) -> TextDocumentSyncKind {
        if let Some(capabilities) = self.server_capabilities.as_ref() {
            if let Some(sync) = capabilities.text_document_sync.as_ref() {
                match sync {
                    TextDocumentSyncCapability::Kind(kind) => {
                        return kind.to_owned();
                    }
                    TextDocumentSyncCapability::Options(_) => {}
                }
            }
        }

        return TextDocumentSyncKind::Full;
    }
}

/// Util Methods
impl LanguageServerClient {
    
    /// Get workspace root using the Workspace Identifier
    /// For example: Cargo.toml can be used to identify a Rust Workspace
    /// This method traverses up to file tree to return the path to the
    /// Workspace root folder
    pub fn get_workspace_root(&mut self, document_path: &Path) -> Option<PathBuf> {
        if let Some(identifier) = &self.workspace_identifier {
            let identifier_os_str = OsStr::new(&identifier);
            let mut current_path = document_path;
            loop {
                let parent_path = current_path.parent();
                if let Some(path) = parent_path {
                    for entry in path.read_dir().expect("Cannot read directory contents") {
                        if let Ok(entry) = entry {
                            if entry.file_name() == identifier_os_str {
                                return Some(entry.path());
                            }
                        }
                    }

                    current_path = path;
                } else {
                    break;
                }
            }
        }
        None
    }
}
