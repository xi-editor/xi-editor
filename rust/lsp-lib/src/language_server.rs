use jsonrpc_lite::{Error, Id, JsonRpc, Params};
use lsp_types::*;
use serde_json;
use serde_json::{to_value, Value};
use std::collections::HashMap;
use std::io::Write;
use std::process;
use url::Url;
use xi_core::ViewIdentifier;
use Callback;

pub type DoucmentURI = Url;

pub struct LanguageServerClient {
    writer: Box<Write + Send>,
    pending: HashMap<usize, Callback>,
    next_id: usize,
    pub is_initialized: bool,
    pub opened_documents: HashMap<ViewIdentifier, DoucmentURI>,
    pub server_capabilities: Option<ServerCapabilities>,
    pub file_extensions: Vec<String>,
}

fn prepare_lsp_json(msg: &Value) -> Result<String, serde_json::error::Error> {
    let request = serde_json::to_string(&msg)?;
    Ok(format!(
        "Content-Length: {}\r\n\r\n{}",
        request.len(),
        request
    ))
}

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
    pub fn new(writer: Box<Write + Send>) -> Self {
        LanguageServerClient {
            writer,
            pending: HashMap::new(),
            next_id: 1,
            is_initialized: false,
            server_capabilities: None,
            opened_documents: HashMap::new(),
            file_extensions: vec!["json".to_string()],
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
            JsonRpc::Success(ref obj) => {
                let mut result = value.get_result().unwrap().to_owned();
                let id = number_from_id(value.get_id().as_ref());
                self.handle_response(id, result);
            }
            JsonRpc::Error(ref obj) => {
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

        self.write(rpc.as_ref());
    }

    pub fn send_notification(&mut self, method: &str, params: Params) {
        let notification = JsonRpc::notification_with_params(method, params);
        self.send_rpc(to_value(&notification).unwrap());
    }
}

impl LanguageServerClient {
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

        eprintln!("\nINIT PARAMS\n: {:?}", init_params);
        let params = Params::from(serde_json::to_value(init_params).unwrap());

        self.send_request("initialize", params, Box::new(on_init));
    }

    pub fn send_did_open(
        &mut self,
        view_id: ViewIdentifier,
        document_uri: Url,
        document_text: String,
    ) {
        eprintln!(
            "DID OPEN CALLED with documentURI {:?} \n document)_text {:?}",
            document_uri, document_text
        );

        self.opened_documents.insert(view_id, document_uri.clone());

        let text_document_did_open_params = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                language_id: "json".to_string(),
                uri: document_uri,
                version: 0,
                text: document_text,
            },
        };

        let params = Params::from(serde_json::to_value(text_document_did_open_params).unwrap());
        self.send_notification("textDocument/didOpen", params);
    }

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


    pub fn send_did_save(
        &mut self,
        view_id: ViewIdentifier,
        _document_text: String
    ) {
        // Add support for sending document text as well. Currently missing in LSP types
        // and is optional in LSP Specification
        let text_document_did_save_params = DidSaveTextDocumentParams {
            text_document: TextDocumentIdentifier {
                uri: self.opened_documents.get(&view_id).unwrap().clone()
            }
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
