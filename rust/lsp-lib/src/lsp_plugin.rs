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

use language_server::LanguageServerClient;
use lsp_types::{
    InitializeResult, Position, Range, TextDocumentContentChangeEvent, TextDocumentSyncKind,
};
use parse_helper;
use serde_json;
use std;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::{BufReader, BufWriter};
use std::option::NoneError;
use std::path::Path;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use types::Error;
use url::Url;
use xi_core::ConfigTable;
use xi_plugin_lib::Error as PluginLibError;
use xi_plugin_lib::{Cache, ChunkCache, Plugin, View};
use xi_rope::rope::RopeDelta;

use types::Config;

pub struct LSPPlugin {
    pub config: Config,
    language_server_clients: HashMap<String, Arc<Mutex<LanguageServerClient>>>,
}

fn get_position_of_offset<C: Cache>(
    view: &mut View<C>,
    offset: usize,
) -> Result<Position, PluginLibError> {
    let line_num = view.line_of_offset(offset)?;
    let line_offset = view.offset_of_line(line_num)?;

    let char_offset: usize = view.get_line(line_num)?[0..(offset - line_offset)]
        .chars()
        .map(char::len_utf16)
        .sum();

    Ok(Position {
        line: line_num as u64,
        character: char_offset as u64,
    })
}

fn get_document_content_changes<C: Cache>(
    delta: Option<&RopeDelta>,
    view: &mut View<C>,
) -> Result<Vec<TextDocumentContentChangeEvent>, PluginLibError> {
    if let Some(delta) = delta {
        if let Some(node) = delta.as_simple_insert() {
            let (interval, _) = delta.summary();
            let text = String::from(node);

            let (start, end) = interval.start_end();
            let text_document_content_change_event = TextDocumentContentChangeEvent {
                range: Some(Range {
                    start: get_position_of_offset(view, start)?,
                    end: get_position_of_offset(view, end)?,
                }),
                range_length: Some((end - start) as u64),
                text,
            };

            Ok(vec![text_document_content_change_event])
        }
        // Or a simple delete
        else if delta.is_simple_delete() {
            let (interval, _) = delta.summary();

            let (start, end) = interval.start_end();

            // Hack around sending VSCode Style Positions to Language Server.
            // See this issue to understand: https://github.com/Microsoft/vscode/issues/23173
            let mut end_position = get_position_of_offset(view, end)?;

            if end_position.character == 0 {
                let mut ep = get_position_of_offset(view, end - 1)?;
                ep.character += 1;
                end_position = ep;
            }

            let text_document_content_change_event = TextDocumentContentChangeEvent {
                range: Some(Range {
                    start: get_position_of_offset(view, start)?,
                    end: end_position,
                }),
                range_length: Some((end - start) as u64),
                text: String::new(),
            };

            Ok(vec![text_document_content_change_event])
        }
        // Send the whole document again if it is not a trivial edit
        else {
            let text_document_content_change_event = TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: view.get_document()?,
            };

            Ok(vec![text_document_content_change_event])
        }
    } else {
        let text_document_content_change_event = TextDocumentContentChangeEvent {
            range: None,
            range_length: None,
            text: view.get_document()?,
        };

        Ok(vec![text_document_content_change_event])
    }
}

pub fn get_workspace_root_uri(
    workspace_identifier: &String,
    document_path: &Path,
) -> Result<Url, Error> {
    let identifier_os_str = OsStr::new(&workspace_identifier);

    let mut current_path = document_path;
    loop {
        let parent_path = current_path.parent();
        if let Some(path) = parent_path {
            for entry in path.read_dir().expect("Cannot read directory contents") {
                if let Ok(entry) = entry {
                    if entry.file_name() == identifier_os_str {
                        let path = entry.path();
                        return Ok(Url::parse(format!("file://{}", path.to_str()?).as_ref())?);
                    };
                }
            }

            current_path = path;
        } else {
            break Err(Error::NoneError);
        }
    }
}

fn start_new_server(
    command: String,
    arguments: Vec<String>,
    file_extensions: Vec<String>,
    workspace_identifier: Option<String>,
    language_id: String,
) -> Result<Arc<Mutex<LanguageServerClient>>, String> {
    let mut process = Command::new(command)
        .env("PATH", "/usr/local/bin")
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Error Occurred");

    let writer = Box::new(BufWriter::new(process.stdin.take().unwrap()));

    let language_server_client = Arc::new(Mutex::new(LanguageServerClient::new(
        writer,
        language_id,
        file_extensions,
        workspace_identifier,
    )));

    {
        let ls_client = language_server_client.clone();
        let mut stdout = process.stdout;

        std::thread::Builder::new()
            .name("STDIN-Looper".to_string())
            .spawn(move || {
                let mut reader = Box::new(BufReader::new(stdout.take().unwrap()));
                loop {
                    match parse_helper::read_message(&mut reader) {
                        Ok(message_str) => {
                            let mut server_locked = ls_client.lock().unwrap();
                            server_locked.handle_message(message_str.as_ref());
                        }
                        Err(err) => eprintln!("Error occurred {:?}", err),
                    };
                }
            });
    }

    Ok(language_server_client)
}

impl LSPPlugin {
    pub fn new(config: Config) -> Self {
        LSPPlugin {
            config,
            language_server_clients: HashMap::new(),
        }
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
        if let Some(language_id) = self.get_language_for_view(view) {
            let workspace_root_uri = {
                let config = &self.config.language_config.get_mut(&language_id).unwrap();
                match &config.workspace_identifier {
                    Some(workspace_identifier) => {
                        let path = view.get_path().clone().unwrap();
                        get_workspace_root_uri(workspace_identifier, path).ok()
                    }
                    None => None,
                }
            };

            if let Some(ls_client) =
                self.get_lsclient_from_workspace_root(language_id, &workspace_root_uri)
            {
                let mut ls_client = ls_client.lock().unwrap();
                let sync_kind = ls_client.get_sync_kind();

                let changes = match sync_kind {
                    TextDocumentSyncKind::None => return,
                    TextDocumentSyncKind::Full => {
                        let text_document_content_change_event = TextDocumentContentChangeEvent {
                            range: None,
                            range_length: None,
                            text: view.get_document().unwrap(),
                        };
                        vec![text_document_content_change_event]
                    }
                    TextDocumentSyncKind::Incremental => {
                        match get_document_content_changes(delta, view) {
                            Ok(result) => result,
                            Err(err) => {
                                eprintln!("Error: {:?} Occured. Sending Whole Doc", err);
                                let text_document_content_change_event =
                                    TextDocumentContentChangeEvent {
                                        range: None,
                                        range_length: None,
                                        text: view.get_document().unwrap(),
                                    };
                                vec![text_document_content_change_event]
                            }
                        }
                    }
                };

                ls_client.send_did_change(view.get_id(), changes, view.rev);
            }
        }
    }

    fn did_save(&mut self, view: &mut View<Self::Cache>, _old: Option<&Path>) {
        eprintln!("saved view {}", view.get_id());

        let document_text = view.get_document().unwrap();

        if let Some(language_id) = self.get_language_for_view(view) {
            let workspace_root_uri = {
                let config = self.config.language_config.get(&language_id).unwrap();
                match &config.workspace_identifier {
                    Some(workspace_identifier) => {
                        let path = view.get_path().clone().unwrap();
                        get_workspace_root_uri(workspace_identifier, path).ok()
                    }
                    None => None,
                }
            };

            let ls_client = self.get_lsclient_from_workspace_root(language_id, &workspace_root_uri);

            if let Some(ls_client) = ls_client {
                let mut ls_client = ls_client.lock().unwrap();
                ls_client.send_did_save(view.get_id(), document_text);
            }
        }
    }

    fn did_close(&mut self, view: &View<Self::Cache>) {
        eprintln!("close view {}", view.get_id());
    }

    fn new_view(&mut self, view: &mut View<Self::Cache>) {
        eprintln!("new view {}", view.get_id());

        let document_text = view.get_document().unwrap();
        let path = view.get_path().clone();
        let view_id = view.get_id().clone();

        if let Some(language_id) = self.get_language_for_view(view) {
            let path = path.unwrap();

            let workspace_root_uri = {
                let config = &self.config.language_config.get_mut(&language_id).unwrap();
                match &config.workspace_identifier {
                    Some(workspace_identifier) => {
                        let path = view.get_path().clone().unwrap();
                        get_workspace_root_uri(workspace_identifier, path).ok()
                    }
                    None => None,
                }
            };

            let ls_client = self.get_lsclient_from_workspace_root(language_id, &workspace_root_uri);

            if let Some(ls_client) = ls_client {
                let mut ls_client = ls_client.lock().unwrap();

                let document_uri =
                    Url::parse(format!("file://{}", path.to_str().unwrap()).as_ref()).unwrap();

                if !ls_client.is_initialized {
                    ls_client.send_initialize(workspace_root_uri, move |ls_client, result| {
                        if result.is_ok() {
                            let init_result: InitializeResult =
                                serde_json::from_value(result.unwrap()).unwrap();

                            eprintln!("INIT RESULT: {:?}", init_result);

                            ls_client.server_capabilities = Some(init_result.capabilities);
                            ls_client.is_initialized = true;
                            ls_client.send_did_open(view_id, document_uri, document_text);
                        }
                    });
                } else {
                    ls_client.send_did_open(view_id, document_uri, document_text);
                }
            }
        }
    }

    fn config_changed(&mut self, _view: &mut View<Self::Cache>, _changes: &ConfigTable) {}
}

/// Utils Methods
impl LSPPlugin {
    fn get_lsclient_from_workspace_root(
        &mut self,
        language_id: String,
        workspace_root: &Option<Url>,
    ) -> Option<Arc<Mutex<LanguageServerClient>>> {
        match workspace_root {
            Some(root) => {
                let root = root.clone().into_string();
                // Find existing client for same root
                let contains = self.language_server_clients.contains_key(&root);

                if !contains {
                    let config = self.config.language_config.get(&language_id).unwrap();

                    let client = start_new_server(
                        config.start_command.clone(),
                        config.start_arguments.clone(),
                        config.extensions.clone(),
                        config.workspace_identifier.clone(),
                        language_id,
                    );

                    match client {
                        Ok(client) => {
                            let client_clone = client.clone();
                            self.language_server_clients.insert(root, client);
                            Some(client_clone)
                        }
                        Err(_) => None,
                    }
                } else {
                    Some(self.language_server_clients.get(&root).unwrap().clone())
                }
            }
            None => {
                let config = self.config.language_config.get(&language_id).unwrap();

                if config.supports_single_file {
                    // We check if a generic client is running. Such a client
                    // supports single files. For example, a json client or
                    // a Python client
                    let contains = self.language_server_clients.contains_key("generic");

                    if !contains {
                        let client = start_new_server(
                            config.start_command.clone(),
                            config.start_arguments.clone(),
                            config.extensions.clone(),
                            config.workspace_identifier.clone(),
                            language_id,
                        );

                        match client {
                            Ok(client) => {
                                let client_clone = client.clone();
                                self.language_server_clients
                                    .insert("generic".to_owned(), client);
                                Some(client_clone)
                            }
                            Err(_) => None,
                        }
                    } else {
                        Some(self.language_server_clients.get("generic").unwrap().clone())
                    }
                } else {
                    None
                }
            }
        }
    }

    fn get_language_for_view(&mut self, view: &View<ChunkCache>) -> Option<String> {
        if let Some(path) = view.get_path().clone() {
            let result: Result<String, NoneError> =
                do catch { path.extension()?.to_str()?.to_string() };

            if let Ok(extension) = result {
                for config in &self.config.language_config {
                    if config.1.extensions.contains(&extension) {
                        return Some(config.0.clone());
                    }
                }
                None
            } else {
                None
            }
        } else {
            None
        }
    }
}
