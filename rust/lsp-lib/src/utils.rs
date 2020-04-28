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

use std::ffi::OsStr;
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use url::Url;
use xi_plugin_lib::{Cache, ChunkCache, CoreProxy, Error as PluginLibError, View};
use xi_rope::rope::RopeDelta;

use crate::conversion_utils::*;
use crate::language_server_client::LanguageServerClient;
use crate::lsp_types::*;
use crate::parse_helper;
use crate::result_queue::ResultQueue;
use crate::types::Error;

/// Get contents changes of a document modeled according to Language Server Protocol
/// given the RopeDelta
pub fn get_document_content_changes<C: Cache>(
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
            let mut end_position = get_position_of_offset(view, end)?;

            // Hack around sending VSCode Style Positions to Language Server.
            // See this issue to understand: https://github.com/Microsoft/vscode/issues/23173
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
pub fn get_change_for_sync_kind(
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
                warn!("Error: {:?} Occured. Sending Whole Doc", err);
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
    workspace_identifier: &str,
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
                        return Url::from_file_path(path).map_err(|_| Error::FileUrlParseError);
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
pub fn start_new_server(
    command: String,
    arguments: Vec<String>,
    file_extensions: Vec<String>,
    language_id: &str,
    core: CoreProxy,
    result_queue: ResultQueue,
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
        result_queue,
        language_id.to_owned(),
        file_extensions,
    )));

    {
        let ls_client = language_server_client.clone();
        let mut stdout = process.stdout;

        // Unwrap to indicate that we want thread to panic on failure
        std::thread::Builder::new()
            .name(format!("{}-lsp-stdout-Looper", language_id))
            .spawn(move || {
                let mut reader = Box::new(BufReader::new(stdout.take().unwrap()));
                loop {
                    match parse_helper::read_message(&mut reader) {
                        Ok(message_str) => {
                            let mut server_locked = ls_client.lock().unwrap();
                            server_locked.handle_message(message_str.as_ref());
                        }
                        Err(err) => error!("Error occurred {:?}", err),
                    };
                }
            })
            .unwrap();
    }

    Ok(language_server_client)
}
