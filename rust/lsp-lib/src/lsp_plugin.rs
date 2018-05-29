use lsp_types::InitializeParams;
use jsonrpc_lite::{self,Params};
use serde_json::Value;
use std;
use std::sync::Arc;
use std::sync::Mutex;
use language_server::LanguageServer;
use jsonrpc_lite::JsonRpc;
use jsonrpc_lite::Id;
use xi_plugin_lib::{Plugin, ChunkCache, View};
use xi_rope::rope::RopeDelta;
use xi_core::{ConfigTable};
use std::path::Path;
use std::process::Command;
use std::process::Stdio;
use parse_helper;
use std::io::{BufWriter, BufReader};
use std::process;
use serde_json;
use lsp_types::ClientCapabilities;


pub struct LSPPlugin {
    language_server_ref: Arc<Mutex<LanguageServer>>,
    file_extensions: Vec<String>
}

impl Clone for LSPPlugin {
    fn clone(&self) -> Self {
        LSPPlugin {
            language_server_ref: self.language_server_ref.clone(),
            file_extensions: self.file_extensions.clone()
        }
    }
}

impl LSPPlugin {

    pub fn new(command: &str, arguments: &[&str]) -> Self {

        eprintln!("command: {}", command);
        eprintln!("arguments: {:?}", arguments);

        let mut process = Command::new(command)
            .env("PATH", "/usr/local/bin")
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("Error Occurred");

        let child_id = Some(process.id());
        eprintln!("child_id: {}", child_id.unwrap());

        let writer = Box::new(BufWriter::new(process.stdin.take().unwrap()));

        let plugin = LSPPlugin {
            language_server_ref : Arc::new(Mutex::new(LanguageServer::new(writer))),
            file_extensions: vec!["json".to_string()]
        };

        {
            let plugin_cloned = plugin.clone();
            std::thread::Builder::new()
                .name("STDIN-Looper".to_string())
                .spawn(move || {
                    let mut reader = Box::new(BufReader::new(process.stdout.take().unwrap()));
                    loop {
                        match parse_helper::read_message(&mut reader) {
                            Ok(message_str) => plugin_cloned.handle_message(message_str.as_ref()),
                            Err(err) => eprintln!("Error occurred {:?}", err),
                        };
                    } 
                });
        }

        plugin
    }

    fn write(&self, msg: &str) {
        let mut lang_server = self.language_server_ref.lock().unwrap();
        lang_server.write(msg);
    }

    pub fn handle_message(&self, message: &str) {

        eprintln!("Value from function: {:?}", message);
        let mut value = JsonRpc::parse(message).unwrap();
        eprintln!("Value from function parsed: {:?}", value);
        
        match value {
            JsonRpc::Request(obj) => eprintln!("client received unexpected request: {:?}", obj),
            JsonRpc::Notification(obj) => eprintln!("recv notification: {:?}", obj),
            JsonRpc::Success(ref obj) => {
                let mut lang_server = self.language_server_ref.lock().unwrap();
                let mut result = value.get_result().unwrap().to_owned();
                let id = number_from_id(value.get_id().as_ref());
                lang_server.handle_response(id, result);
            }
            JsonRpc::Error(ref obj) => {
                let mut lang_server = self.language_server_ref.lock().unwrap();
                let mut error = value.get_error().unwrap().to_owned();
                let id = number_from_id(value.get_id().as_ref());
                lang_server.handle_error(id, error);
            }
        };
    }

    /// Sends a JSON-RPC request message with the provided method and parameters.
    /// `completion` should be a callback which will be executed with the server's response.
    pub fn send_request<CB>(&self, method: &str, params: Params, completion: CB)
        where CB: 'static + Send + FnOnce(Result<Value, jsonrpc_lite::Error>) {
            let mut inner = self.language_server_ref.lock().unwrap();
            inner.send_request(method, params, Box::new(completion));
    }

    /// Sends a JSON-RPC notification message with the provided method and parameters.
    pub fn send_notification(&self, method: &str, params: Params) {
        let mut inner = self.language_server_ref.lock().unwrap();
        inner.send_notification(method, params);
    }
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

impl Plugin for LSPPlugin {
    type Cache = ChunkCache;

    fn update(&mut self, view: &mut View<Self::Cache>, delta: Option<&RopeDelta>,
              _edit_type: String, _author: String) {}

    fn did_save(&mut self, view: &mut View<Self::Cache>, _old: Option<&Path>) {
        eprintln!("saved view {}", view.get_id());
    }

    fn did_close(&mut self, view: &View<Self::Cache>) {
        eprintln!("close view {}", view.get_id());
    }

    fn new_view(&mut self, view: &mut View<Self::Cache>) {
        eprintln!("new view {}", view.get_id());

        let name = view.get_path();
    
        if let Some(path) = name {
            let extension = path.extension().unwrap().to_str().unwrap().to_string();
            if self.file_extensions.contains(&extension) {
                eprintln!("json file opened");
                self.send_initialize();
            }
        }
    }

    fn config_changed(&mut self, _view: &mut View<Self::Cache>, _changes: &ConfigTable) {}
}


//Utils Methods for sending
impl LSPPlugin {

    pub fn send_initialize(&mut self) {
        
        let client_capabilities = ClientCapabilities::default();

        let init_params = InitializeParams {
            process_id: Some(process::id() as u64),
            root_uri: None,
            root_path: None,
            initialization_options: None,
            capabilities: client_capabilities,
            trace: None
        };

        let params = Params::from(serde_json::to_value(init_params).unwrap());
        self.send_request("initialize", params, |result| {
            eprintln!("Received Response: {:?}", result);
        });
    }
}