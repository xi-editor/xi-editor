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

//! Implementation of Language Server Plugin

use language_server_client::LanguageServerClient;
use lsp_types::Hover;
use lsp_types::{
    InitializeResult, Position, Range, TextDocumentContentChangeEvent, TextDocumentSyncKind,
};
use parse_helper;
use serde_json;
use std;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use types::Error;
use url::Url;
use xi_core::ConfigTable;
use xi_core::ViewId;
use xi_plugin_lib::CoreProxy;
>>>>>>> aa07428... Initial Stage Hover RPC
use xi_plugin_lib::Error as PluginLibError;
use xi_plugin_lib::{
    Cache, ChunkCache, HoverResult, Plugin, Position as CorePosition, Range as CoreRange, View,
};
use xi_rope::rope::RopeDelta;

use types::Config;

pub struct ViewInfo {
    version: u64,
    ls_identifier: String,
}

/// Represents the state of the Language Server Plugin
pub struct LspPlugin {
    pub config: Config,
    view_info: HashMap<ViewId, ViewInfo>,
    core: Option<CoreProxy>,
    language_server_clients: HashMap<String, Arc<Mutex<LanguageServerClient>>>,
}

/// Counts the number of utf-16 code units in the given string.
fn count_utf16(s: &str) -> usize {
    let mut utf16_count = 0;
    for &b in s.as_bytes() {
        if (b as i8) >= -0x40 {
            utf16_count += 1;
        }
        if b >= 0xf0 {
            utf16_count += 1;
        }
    }
    utf16_count
}

/// Get LSP Style Utf-16 based position given the xi-core style utf-8 offset
fn get_position_of_offset<C: Cache>(
    view: &mut View<C>,
    offset: usize,
) -> Result<Position, PluginLibError> {
    let line_num = view.line_of_offset(offset)?;
    let line_offset = view.offset_of_line(line_num)?;

    let char_offset: usize = count_utf16(&(view.get_line(line_num)?[0..(offset - line_offset)]));

    Ok(Position {
        line: line_num as u64,
        character: char_offset as u64,
    })
}

fn lsp_position_from_core_position<C: Cache>(
    view: &mut View<C>,
    position: CorePosition
) -> Result<Position, PluginLibError> {

    match position {
        CorePosition::Utf8LineChar {line, character} => {
            let line_text = view.get_line(line)?;
            let char_offset: usize = line_text[0..character]
                        .chars()
                        .map(char::len_utf16)
                        .sum();

            Ok(Position{
                line: line as u64,
                character: char_offset as u64
            })
        },
        CorePosition::Utf16LineChar {line, character} => Ok(Position{
            line: line as u64,
            character: character as u64,
        }),
        CorePosition::Utf8Offset(offset) => get_position_of_offset(view, offset)
    }
}

/// Get xi-core format utf-8 offset using the the LSP Position
#[allow(dead_code)]
fn get_offset_of_position<C: Cache>(
    view: &mut View<C>,
    position: Position,
) -> Result<usize, PluginLibError> {
    let line_offset = view.offset_of_line(position.line as usize)?;

    let char_lengths = view
        .get_line(line_offset)?
        .chars()
        .map(|c| (c.len_utf8(), c.len_utf16()));

    let mut cur_len_utf16 = 0;
    let mut cur_len_utf8 = 0;

    for length in char_lengths {
        cur_len_utf16 += length.1;
        cur_len_utf8 += length.0;
        if cur_len_utf16 == (position.character as usize) {
            return Ok(line_offset + cur_len_utf8);
        }
    }

    Err(PluginLibError::Other(String::from(
        "Cannot convert to offset",
    )))
}

/// Get contents changes of a document modeled according to Language Server Protocol
/// given the RopeDelta
fn get_document_content_changes<C: Cache>(
    delta: Option<&RopeDelta>,
    view: &mut View<C>,
) -> Result<Vec<TextDocumentContentChangeEvent>, PluginLibError> {
    if let Some(delta) = delta {
        let (interval, _) = delta.summary();
        let (start, end) = interval.start_end();

        // TODO: Handle more trivial cases like typing when there's a selection or transpose
        if let Some(node) = delta.as_simple_insert() {
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

            return Ok(vec![text_document_content_change_event]);
        }
        // Or a simple delete
        else if delta.is_simple_delete() {
            // Hack around sending VSCode Style Positions to Language Server.
            // See this issue to understand: https://github.com/Microsoft/vscode/issues/23173
            let mut end_position = get_position_of_offset(view, end)?;

            if end_position.character == 0 {
                // There is an assumption here that the line separator character is exactly
                // 1 byte wide which is true for "\n" but it will be an issue if they are not
                // for example for u+2028
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

            return Ok(vec![text_document_content_change_event]);
        }
    }

    let text_document_content_change_event = TextDocumentContentChangeEvent {
        range: None,
        range_length: None,
        text: view.get_document()?,
    };

    Ok(vec![text_document_content_change_event])
}

/// Get changes to be sent to server depending upon the type of Sync supported
/// by server
fn get_change_for_sync_kind(
    sync_kind: TextDocumentSyncKind,
    view: &mut View<ChunkCache>,
    delta: Option<&RopeDelta>,
) -> Option<Vec<TextDocumentContentChangeEvent>> {
    match sync_kind {
        TextDocumentSyncKind::None => None,
        TextDocumentSyncKind::Full => {
            let text_document_content_change_event = TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: view.get_document().unwrap(),
            };
            Some(vec![text_document_content_change_event])
        }
        TextDocumentSyncKind::Incremental => match get_document_content_changes(delta, view) {
            Ok(result) => Some(result),
            Err(err) => {
                eprintln!("Error: {:?} Occured. Sending Whole Doc", err);
                let text_document_content_change_event = TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: view.get_document().unwrap(),
                };
                Some(vec![text_document_content_change_event])
            }
        },
    }
}

/// Get workspace root using the Workspace Identifier and the opened document path
/// For example: Cargo.toml can be used to identify a Rust Workspace
/// This method traverses up to file tree to return the path to the Workspace root folder
pub fn get_workspace_root_uri(
    workspace_identifier: &String,
    document_path: &Path,
) -> Result<Url, Error> {
    let identifier_os_str = OsStr::new(&workspace_identifier);

    let mut current_path = document_path;
    loop {
        let parent_path = current_path.parent();
        if let Some(path) = parent_path {
            for entry in path.read_dir()? {
                if let Ok(entry) = entry {
                    if entry.file_name() == identifier_os_str {
                        return Url::from_directory_path(path).map_err(|_| Error::FileUrlParseError);
                    };
                }
            }
            current_path = path;
        } else {
            break Err(Error::PathError);
        }
    }
}

/// Start a new Language Server Process by spawning a process given the parameters
/// Returns a Arc to the Language Server Client which abstracts connection to the
/// server
fn start_new_server(
    command: String,
    arguments: Vec<String>,
    file_extensions: Vec<String>,
    language_id: String,
    core: CoreProxy,
) -> Result<Arc<Mutex<LanguageServerClient>>, String> {
    let mut process = Command::new(command)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Error Occurred");

    let writer = Box::new(BufWriter::new(process.stdin.take().unwrap()));

    let language_server_client = Arc::new(Mutex::new(LanguageServerClient::new(
        writer,
        core,
        language_id.clone(),
        file_extensions,
    )));

    {
        let ls_client = language_server_client.clone();
        let mut stdout = process.stdout;

        // Unwrap to indicate that we want thread to panic on failure
        std::thread::Builder::new()
            .name(format!("{}-lsp-stdin-Looper", language_id))
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
            })
            .unwrap();
    }

    Ok(language_server_client)
}

impl LspPlugin {
    pub fn new(config: Config) -> Self {
        LspPlugin {
            config,
            core: None,
            view_info: HashMap::new(),
            language_server_clients: HashMap::new(),
        }
    }
}

impl Plugin for LspPlugin {
    type Cache = ChunkCache;

    fn initialize(&mut self, core: CoreProxy) {
        self.core = Some(core)
    }

    fn update(
        &mut self,
        view: &mut View<Self::Cache>,
        delta: Option<&RopeDelta>,
        _edit_type: String,
        _author: String,
    ) {
        let view_info = self.view_info.get_mut(&view.get_id());
        if let Some(view_info) = view_info {
            // This won't fail since we definitely have a client for the given
            // client identifier
            let ls_client = self
                .language_server_clients
                .get(&view_info.ls_identifier)
                .unwrap();
            let mut ls_client = ls_client.lock().unwrap();

            let sync_kind = ls_client.get_sync_kind();

            view_info.version += 1;
            if let Some(changes) = get_change_for_sync_kind(sync_kind, view, delta) {
                ls_client.send_did_change(view.get_id(), changes, view_info.version);
            }
        }
    }

    fn did_save(&mut self, view: &mut View<Self::Cache>, _old: Option<&Path>) {
        eprintln!("saved view {}", view.get_id());

        let document_text = view.get_document().unwrap();

        let client_identifier = self.view_info.get(&view.get_id());
        if let Some(view_info) = client_identifier {
            // This won't fail since we definitely have a client for the given
            // client identifier
            let ls_client = self
                .language_server_clients
                .get(&view_info.ls_identifier)
                .unwrap();
            let mut ls_client = ls_client.lock().unwrap();
            ls_client.send_did_save(view.get_id(), document_text);
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

        // TODO: Use Language Idenitifier assigned by core when the
        // implementation is settled
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

            let result = self.get_lsclient_from_workspace_root(language_id, &workspace_root_uri);

            if let Some((identifier, ls_client)) = result {
                self.view_info.insert(
                    view.get_id(),
                    ViewInfo {
                        version: 0,
                        ls_identifier: identifier,
                    },
                );
                let mut ls_client = ls_client.lock().unwrap();

                let document_uri = Url::from_file_path(path).unwrap();

                if !ls_client.is_initialized {
                    ls_client.send_initialize(workspace_root_uri, move |ls_client, result| {
                        if let Ok(result) = result {
                            let init_result: InitializeResult =
                                serde_json::from_value(result).unwrap();

                            eprintln!("Init Result: {:?}", init_result);

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

    fn get_hover_definition(
        &mut self,
        view: &mut View<Self::Cache>,
        request_id: usize,
        position: CorePosition,
    ) {
        let view_info = self.view_info.get_mut(&view.get_id());
        if let Some(view_info) = view_info {
            // This won't fail since we definitely have a client for the given
            // client identifier
            let ls_client = self
                .language_server_clients
                .get(&view_info.ls_identifier)
                .unwrap();

            let position_ls = lsp_position_from_core_position(view, position);

            match position_ls {
                Ok(position) => {
                    let mut ls_client = ls_client.lock().unwrap();
                    ls_client.request_hover_definition(
                        view.get_id(),
                        position,
                        move |ls_client, result| match result {
                            Ok(result) => {
                                let hover: Hover = serde_json::from_value(result).unwrap();

                                let hover_result = HoverResult {
                                    request_id,
                                    content: String::new(),
                                    range: hover.range.and_then(|range| {
                                        Some(CoreRange {
                                            start: CorePosition::Utf16LineChar {
                                                line: range.start.line as usize,
                                                character: range.start.character as usize,
                                            },
                                            end: CorePosition::Utf16LineChar {
                                                line: range.start.line as usize,
                                                character: range.start.character as usize,
                                            },
                                        })
                                    }),
                                };

                                ls_client
                                    .core
                                    .display_hover_result(request_id, Some(hover_result));
                            }
                            Err(err) => {
                                eprintln!("Hover Response from LSP Error: {:?}", err);
                                ls_client.core.display_hover_result(request_id, None);
                            }
                        },
                    );
                }
                Err(_error) => {}
            };
        }
    }
}

/// Util Methods
impl LspPlugin {
    /// Get the Language Server Client given the Workspace root
    /// This method checks if a language server is running at the specified root
    /// and returns it else it tries to spawn a new language server and returns a
    /// Arc reference to it
    fn get_lsclient_from_workspace_root(
        &mut self,
        language_id: String,
        workspace_root: &Option<Url>,
    ) -> Option<(String, Arc<Mutex<LanguageServerClient>>)> {
        workspace_root
            .clone()
            .and_then(|r| Some(r.clone().into_string()))
            .or_else(|| {
                let config = self.config.language_config.get(&language_id).unwrap();
                if config.supports_single_file {
                    // A generic client is the one that supports single files i.e.
                    // Non-Workspace projects as well
                    Some(String::from("generic"))
                } else {
                    None
                }
            })
            .and_then(|language_server_identifier| {
                let contains = self
                    .language_server_clients
                    .contains_key(&language_server_identifier);

                if contains {
                    let client = self
                        .language_server_clients
                        .get(&language_server_identifier)
                        .unwrap()
                        .clone();

                    Some((language_server_identifier, client))
                } else {
                    let config = self.config.language_config.get(&language_id).unwrap();

                    let client = start_new_server(
                        config.start_command.clone(),
                        config.start_arguments.clone(),
                        config.extensions.clone(),
                        language_id.clone(),
                        // Unwrap is safe
                        self.core.clone().unwrap(),
                    );

                    match client {
                        Ok(client) => {
                            let client_clone = client.clone();
                            self.language_server_clients
                                .insert(language_server_identifier.clone(), client);

                            Some((language_server_identifier, client_clone))
                        }
                        Err(err) => {
                            eprintln!(
                                "Error occured while starting server for Language: {}: {:?}",
                                language_id, err
                            );
                            None
                        }
                    }
                }
            })
    }

    /// Tries to get language for the View using the extension of the document.
    /// Only searches for the languages supported by the Language Plugin as
    /// defined in the config
    fn get_language_for_view(&mut self, view: &View<ChunkCache>) -> Option<String> {
        view.get_path()
            .and_then(|path| path.extension())
            .and_then(|extension| extension.to_str())
            .and_then(|extension_str| {
                for (lang, config) in &self.config.language_config {
                    if config.extensions.iter().any(|x| x == extension_str) {
                        return Some(lang.clone());
                    }
                }
                None
            })
    }
}
