use jsonrpc_lite::Error;
use jsonrpc_lite::{self, Params};
use language_server::LanguageServerClient;
use lsp_types::ClientCapabilities;
use lsp_types::InitializeParams;
use parse_helper;
use serde_json;
use serde_json::Value;
use std;
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::process;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use url::Url;
use xi_core::ConfigTable;
use xi_plugin_lib::{ChunkCache, Plugin, View};
use xi_rope::rope::RopeDelta;

pub struct LSPPlugin {
    language_server_ref: Arc<Mutex<LanguageServerClient>>,
    is_initialized: bool,
    file_extensions: Vec<String>,
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
            language_server_ref: Arc::new(Mutex::new(LanguageServerClient::new(writer))),
            is_initialized: false,
            file_extensions: vec!["json".to_string()],
        };

        {
            let server_ref = plugin.language_server_ref.clone();
            std::thread::Builder::new()
                .name("STDIN-Looper".to_string())
                .spawn(move || {
                    let mut reader = Box::new(BufReader::new(process.stdout.take().unwrap()));
                    loop {
                        match parse_helper::read_message(&mut reader) {
                            Ok(message_str) => {
                                let mut server_locked = server_ref.lock().unwrap();
                                server_locked.handle_message(message_str.as_ref());
                            }
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

    /// Sends a JSON-RPC request message with the provided method and parameters.
    /// `completion` should be a callback which will be executed with the server's response.
    pub fn send_request<CB>(
        &self,
        method: &str,
        params: Params,
        completion: CB,
    ) where
        CB: 'static + Send + FnOnce(Result<Value, jsonrpc_lite::Error>),
    {
        let mut inner = self.language_server_ref.lock().unwrap();
        inner.send_request(method, params, Box::new(completion));
    }

    /// Sends a JSON-RPC notification message with the provided method and parameters.
    pub fn send_notification(&self, method: &str, params: Params) {
        let mut inner = self.language_server_ref.lock().unwrap();
        inner.send_notification(method, params);
    }
}

pub struct XiLSPPlugin(Arc<Mutex<LSPPlugin>>);

impl XiLSPPlugin {
    pub fn new(command: &str, arguments: &[&str]) -> Self {
        let plugin = LSPPlugin::new(command, arguments);
        XiLSPPlugin(Arc::new(Mutex::new(plugin)))
    }
}

impl Plugin for XiLSPPlugin {
    type Cache = ChunkCache;

    fn update(
        &mut self,
        view: &mut View<Self::Cache>,
        delta: Option<&RopeDelta>,
        _edit_type: String,
        _author: String,
    ) {
    }

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

            let mut plugin_ref = self.0.lock().unwrap();

            if plugin_ref.file_extensions.contains(&extension) {
                eprintln!("json file opened");
                //let documentURI = Url::parse(format!("file://{}", path.to_str().unwrap()).as_ref()).unwrap();

                let plugin_arc_clone = self.0.clone();
                if !plugin_ref.is_initialized {
                    plugin_ref.send_initialize(
                        None,
                        Some(move |result: Result<Value, Error>| {
                            let mut plugin_ref = plugin_arc_clone.lock().unwrap();

                            if result.is_ok() {
                                plugin_ref.is_initialized = true;
                            }
                            plugin_ref.send_did_open();
                        }),
                    );

                } else {
                    plugin_ref.send_did_open();
                }
            }
        }
    }

    fn config_changed(&mut self, _view: &mut View<Self::Cache>, _changes: &ConfigTable) {}
}

//Utils Methods for sending
impl LSPPlugin {
    pub fn send_initialize<F>(&mut self, root_uri: Option<Url>, on_init: Option<F>)
    where
        F: 'static + Send + FnOnce(Result<Value, Error>) -> (),
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

        self.send_request("initialize", params, |result| {
            if let Some(f) = on_init {
                f(result.clone());
            }
        });
    }

    pub fn send_did_open(&mut self) {
        eprintln!("DID OPEN CALLED")
    }

    pub fn request_diagonostics(&mut self) {}
}
