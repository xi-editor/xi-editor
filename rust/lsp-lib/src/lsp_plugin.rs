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

//! Implementation of Language Server Plugin

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use url::Url;
use xi_plugin_lib::{ChunkCache, CoreProxy, Plugin, View};
use xi_rope::rope::RopeDelta;

use crate::conversion_utils::*;
use crate::language_server_client::LanguageServerClient;
use crate::lsp_types::*;
use crate::result_queue::ResultQueue;
use crate::types::{Config, LanguageResponseError, LspResponse};
use crate::utils::*;
use crate::xi_core::{ConfigTable, ViewId};

pub struct ViewInfo {
    version: u64,
    ls_identifier: String,
}

/// Represents the state of the Language Server Plugin
pub struct LspPlugin {
    pub config: Config,
    view_info: HashMap<ViewId, ViewInfo>,
    core: Option<CoreProxy>,
    result_queue: ResultQueue,
    language_server_clients: HashMap<String, Arc<Mutex<LanguageServerClient>>>,
}

impl LspPlugin {
    pub fn new(config: Config) -> Self {
        LspPlugin {
            config,
            core: None,
            result_queue: ResultQueue::new(),
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
            let ls_client = &self.language_server_clients[&view_info.ls_identifier];
            let mut ls_client = ls_client.lock().unwrap();

            let sync_kind = ls_client.get_sync_kind();
            view_info.version += 1;
            if let Some(changes) = get_change_for_sync_kind(sync_kind, view, delta) {
                ls_client.send_did_change(view.get_id(), changes, view_info.version);
            }
        }
    }

    fn did_save(&mut self, view: &mut View<Self::Cache>, _old: Option<&Path>) {
        trace!("saved view {}", view.get_id());

        let document_text = view.get_document().unwrap();
        self.with_language_server_for_view(view, |ls_client| {
            ls_client.send_did_save(view.get_id(), &document_text);
        });
    }

    fn did_close(&mut self, view: &View<Self::Cache>) {
        trace!("close view {}", view.get_id());

        self.with_language_server_for_view(view, |ls_client| {
            ls_client.send_did_close(view.get_id());
        });
    }

    fn new_view(&mut self, view: &mut View<Self::Cache>) {
        trace!("new view {}", view.get_id());

        let document_text = view.get_document().unwrap();
        let path = view.get_path();
        let view_id = view.get_id();

        // TODO: Use Language Idenitifier assigned by core when the
        // implementation is settled
        if let Some(language_id) = self.get_language_for_view(view) {
            let path = path.unwrap();

            let workspace_root_uri = {
                let config = &self.config.language_config.get_mut(&language_id).unwrap();

                config.workspace_identifier.clone().and_then(|identifier| {
                    let path = view.get_path().unwrap();
                    let q = get_workspace_root_uri(&identifier, path);
                    q.ok()
                })
            };

            let result = self.get_lsclient_from_workspace_root(&language_id, &workspace_root_uri);

            if let Some((identifier, ls_client)) = result {
                self.view_info
                    .insert(view.get_id(), ViewInfo { version: 0, ls_identifier: identifier });
                let mut ls_client = ls_client.lock().unwrap();

                let document_uri = Url::from_file_path(path).unwrap();

                if !ls_client.is_initialized {
                    ls_client.send_initialize(workspace_root_uri, move |ls_client, result| {
                        if let Ok(result) = result {
                            let init_result: InitializeResult =
                                serde_json::from_value(result).unwrap();

                            debug!("Init Result: {:?}", init_result);

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

    fn get_hover(&mut self, view: &mut View<Self::Cache>, request_id: usize, position: usize) {
        let view_id = view.get_id();
        let position_ls = get_position_of_offset(view, position);

        self.with_language_server_for_view(view, |ls_client| match position_ls {
            Ok(position) => ls_client.request_hover(view_id, position, move |ls_client, result| {
                let res = result
                    .map_err(|e| LanguageResponseError::LanguageServerError(format!("{:?}", e)))
                    .and_then(|h| {
                        let hover: Option<Hover> = serde_json::from_value(h).unwrap();
                        hover.ok_or(LanguageResponseError::NullResponse)
                    });

                ls_client.result_queue.push_result(request_id, LspResponse::Hover(res));
                ls_client.core.schedule_idle(view_id);
            }),
            Err(err) => {
                ls_client.result_queue.push_result(request_id, LspResponse::Hover(Err(err.into())));
                ls_client.core.schedule_idle(view_id);
            }
        });
    }

    fn idle(&mut self, view: &mut View<Self::Cache>) {
        let result = self.result_queue.pop_result();
        if let Some((request_id, reponse)) = result {
            match reponse {
                LspResponse::Hover(res) => {
                    let res =
                        res.and_then(|h| core_hover_from_hover(view, h)).map_err(|e| e.into());
                    self.with_language_server_for_view(view, |ls_client| {
                        ls_client.core.display_hover(view.get_id(), request_id, &res)
                    });
                }
            }
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
        language_id: &str,
        workspace_root: &Option<Url>,
    ) -> Option<(String, Arc<Mutex<LanguageServerClient>>)> {
        workspace_root
            .clone()
            .map(|r| r.into_string())
            .or_else(|| {
                let config = &self.config.language_config[language_id];
                if config.supports_single_file {
                    // A generic client is the one that supports single files i.e.
                    // Non-Workspace projects as well
                    Some(String::from("generic"))
                } else {
                    None
                }
            })
            .and_then(|language_server_identifier| {
                let contains =
                    self.language_server_clients.contains_key(&language_server_identifier);

                if contains {
                    let client = self.language_server_clients[&language_server_identifier].clone();

                    Some((language_server_identifier, client))
                } else {
                    let config = &self.config.language_config[language_id];
                    let client = start_new_server(
                        config.start_command.clone(),
                        config.start_arguments.clone(),
                        config.extensions.clone(),
                        language_id,
                        // Unwrap is safe
                        self.core.clone().unwrap(),
                        self.result_queue.clone(),
                    );

                    match client {
                        Ok(client) => {
                            let client_clone = client.clone();
                            self.language_server_clients
                                .insert(language_server_identifier.clone(), client);

                            Some((language_server_identifier, client_clone))
                        }
                        Err(err) => {
                            error!(
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

    fn with_language_server_for_view<F, R>(&mut self, view: &View<ChunkCache>, f: F) -> Option<R>
    where
        F: FnOnce(&mut LanguageServerClient) -> R,
    {
        let view_info = self.view_info.get_mut(&view.get_id())?;
        let ls_client_arc = &self.language_server_clients[&view_info.ls_identifier];
        let mut ls_client = ls_client_arc.lock().unwrap();
        Some(f(&mut ls_client))
    }
}
