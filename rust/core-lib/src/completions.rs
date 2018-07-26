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

use std::collections::{HashMap, HashSet};

use xi_rpc::RemoteError;

use plugins::PluginId;
use plugins::rpc::{CompletionItem, CompletionResponse};

#[derive(Debug)]
struct CompletionSource {
    id: PluginId,
    is_incomplete: bool,
    can_resolve: bool,
}

/// A representation of the state of an autocomplete dialog.
///
/// This state is updated as results are returned from various sources,
/// and as the user continues to type. When the state has changes it
/// marks itself as dirty, indicating that the client should be updated.
#[derive(Debug)]
pub(crate) struct CompletionState {
    /// threaded through with requests, to ensure we don't handle stale
    /// responses.
    pub(crate) id: usize,
    pub(crate) is_dirty: bool,
    /// offset of cursor at rev of original request
    pub(crate) pos: usize,
    pub(crate) is_cancelled: bool,
    sources: HashMap<PluginId, CompletionSource>,
    /// rev of original request?
    rev: u64,
    /// sorted
    items: Vec<(CompletionItem, PluginId)>,
    selected: usize,
    /// outstanding requests related to this completion
    pending: HashSet<PluginId>,
}

impl CompletionState {
    pub(crate) fn new(id: usize, pos: usize, rev: u64) -> Self {
        CompletionState {
            id,
            is_dirty: false,
            sources: HashMap::new(),
            rev,
            pos,
            items: Vec::new(),
            selected: 0,
            pending: HashSet::new(),
            is_cancelled: false,
        }
    }

    pub(crate) fn add_pending(&mut self, plugin: PluginId) {
        self.pending.insert(plugin);
    }

    pub(crate) fn cancel(&mut self) {
        self.is_cancelled = true;
        self.is_dirty = true
    }

    pub(crate) fn handle_response(&mut self, plugin: PluginId,
                                  response: Result<CompletionResponse, RemoteError>)
    {
        let is_last_response = self.pending.remove(&plugin)
            && self.pending.is_empty();
        match response {
            Ok(response) => {
                let source = CompletionSource {
                    id: plugin,
                    is_incomplete: response.is_incomplete,
                    can_resolve: response.can_resolve,
                };
                self.sources.insert(plugin, source);
                eprintln!("got completions {:?}", response.items.iter().map(|i| i.label.clone()).collect::<Vec<_>>());
                self.items.extend(response.items.into_iter().map(|i| (i, plugin)));
                self.sort_items();
                self.is_dirty = true;
            }
            Err(e) => {
                eprintln!("completions error: {:?}: {:?}", plugin, e);
                if is_last_response {
                    self.is_dirty = true;
                }
            }
        }
    }

    /// The bits of our state that are used when updating the client:
    /// The start offset of the text being completed, the index of the selected
    /// completion item, and the completions themselves.
    pub(crate) fn client_completions(&self) -> (usize, usize, Vec<ClientCompletionItem>) {
        let items = self.items.iter()
            .map(|(item, _)| item.get_client_item())
            .collect::<Vec<_>>();
        (self.pos, self.selected, items)
    }

    fn sort_items(&mut self) {
        //TODO: update any non-zero selection
        self.items.sort_by(|a, b| a.0.sort_key().cmp(b.0.sort_key()))
    }
}

#[derive(Debug, Serialize)]
pub struct ClientCompletionItem<'a> {
    label: &'a str,
    //kind: Option<usize>,
    detail: Option<&'a str>,
    documentation: Option<&'a str>,
}

impl CompletionItem {
    // just for testing
    pub fn with_label<S: AsRef<str>>(label: S) -> Self {
        let mut item = CompletionItem::default();
        item.label = label.as_ref().into();
        item
    }

    fn sort_key(&self) -> &str {
        self.sort_text.as_ref()
            .map(String::as_str)
            .unwrap_or(self.label.as_str())
    }

    fn get_client_item(&self) -> ClientCompletionItem {
        ClientCompletionItem {
            label: self.label.as_str(),
            //kind: self.kind.clone(),
            detail: self.detail.as_ref().map(|s| s.as_str()),
            documentation: self.documentation.as_ref().map(|s| s.as_str()),
        }
    }
}
