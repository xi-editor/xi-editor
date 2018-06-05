use language_server::LanguageServerClient;
use lsp_types::{
    InitializeResult, Position, Range, TextDocumentContentChangeEvent, TextDocumentSyncKind,
};
use parse_helper;
use serde_json;
use std;
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use url::Url;
use xi_core::ConfigTable;
use xi_plugin_lib::Error;
use xi_plugin_lib::{Cache, ChunkCache, Plugin, View};
use xi_rope::rope::RopeDelta;

pub struct LSPPlugin(Arc<Mutex<LanguageServerClient>>);

fn get_position_of_offset<C: Cache>(view: &mut View<C>, offset: usize) -> Result<Position, Error> {
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
    d: &RopeDelta,
    view: &mut View<C>,
) -> Result<Vec<TextDocumentContentChangeEvent>, Error> {
    if let Some(node) = d.as_simple_insert() {
        let (interval, _) = d.summary();
        let text = String::from(node);

        eprintln!("Text Inserted: {} ", text);

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
    else if d.is_simple_delete() {
        let (interval, _) = d.summary();

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

        let plugin = LSPPlugin(Arc::new(Mutex::new(LanguageServerClient::new(writer))));

        {
            let server_ref = plugin.0.clone();
            let mut stdout = process.stdout;

            std::thread::Builder::new()
                .name("STDIN-Looper".to_string())
                .spawn(move || {
                    let mut reader = Box::new(BufReader::new(stdout.take().unwrap()));
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
        if let Some(d) = delta {
            // Check if the delta is a simple insert operation
            //let changes = try!( );

            let mut ls_client = self.0.lock().unwrap();
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
                TextDocumentSyncKind::Incremental => match get_document_content_changes(d, view) {
                    Ok(result) => result,
                    Err(err) => {
                        eprintln!("Error Occured. Sending Whole Doc");
                        let text_document_content_change_event = TextDocumentContentChangeEvent {
                            range: None,
                            range_length: None,
                            text: view.get_document().unwrap(),
                        };
                        vec![text_document_content_change_event]
                    }
                },
            };

            ls_client.send_did_change(view.get_id(), changes, view.rev);
        }
    }

    fn did_save(&mut self, view: &mut View<Self::Cache>, _old: Option<&Path>) {
        eprintln!("saved view {}", view.get_id());

        let document_text = view.get_document().unwrap();
        let mut ls_client = self.0.lock().unwrap();
        ls_client.send_did_save(view.get_id(), document_text);
        
    }

    fn did_close(&mut self, view: &View<Self::Cache>) {
        eprintln!("close view {}", view.get_id());
    }

    fn new_view(&mut self, view: &mut View<Self::Cache>) {
        eprintln!("new view {}", view.get_id());

        let document_text = view.get_document().unwrap();
        let path = view.get_path().clone();
        let view_id = view.get_id().clone();

        if let Some(file_path) = path {
            let extension = file_path.extension().unwrap().to_str().unwrap().to_string();

            let mut ls_client = self.0.lock().unwrap();

            if ls_client.file_extensions.contains(&extension) {
                eprintln!("json file opened");
                let document_uri =
                    Url::parse(format!("file://{}", file_path.to_str().unwrap()).as_ref()).unwrap();

                if !ls_client.is_initialized {
                    ls_client.send_initialize(None, move |ls_client, result| {
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
