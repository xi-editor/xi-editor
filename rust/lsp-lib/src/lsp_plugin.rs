use language_server::LanguageServerClient;
use parse_helper;
use url::Url;
use std;
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use xi_core::ConfigTable;
use xi_plugin_lib::{ChunkCache, Plugin, View};
use xi_rope::rope::RopeDelta;

pub struct LSPPlugin(Arc<Mutex<LanguageServerClient>>);

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

        let plugin = LSPPlugin(Arc::new(Mutex::new(LanguageServerClient::new(writer))));

        {
            let server_ref = plugin.0.clone();
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

}

impl Plugin for LSPPlugin {
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

            let mut ls_client = self.0.lock().unwrap();

            if ls_client.file_extensions.contains(&extension) {
                eprintln!("json file opened");
                let document_uri = Url::parse(format!("file://{}", path.to_str().unwrap()).as_ref()).unwrap();

                if !ls_client.is_initialized {
                    ls_client.send_initialize( None, move |ls_client, result| {
                            if result.is_ok() {
                                ls_client.is_initialized = true;
                                ls_client.send_did_open(document_uri);
                            }
                        },
                    );

                } else {
                    ls_client.send_did_open(document_uri);
                }
            }
        }
    }

    fn config_changed(&mut self, _view: &mut View<Self::Cache>, _changes: &ConfigTable) {}
}
