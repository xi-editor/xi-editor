// Copyright 2017 Google Inc. All rights reserved.
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

//! RPC types, corresponding to protocol requests, notifications & responses.


//TODO: At the moment (May 08, 2017) this is all very much in flux.
// At some point, it will be stabalized and then perhaps will live in another crate,
// shared with the plugin lib.

//TODO: very likely this should be merged with PluginDescription
/// Describes an available plugin to the client.
#[derive(Serialize, Deserialize, Debug)]
pub struct ClientPluginInfo {
    pub name: String,
    pub running: bool,
}

/// A simple update, sent to a plugin.
#[derive(Serialize, Deserialize, Debug)]
pub struct PluginUpdate {
    start: usize,
    end: usize,
    new_len: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    rev: usize,
    edit_type: String,
    author: String,
}

impl PluginUpdate {
    pub fn new(start: usize, end: usize, new_len: usize, rev: usize,
               text: Option<String>, edit_type: String, author: String) -> Self {
        PluginUpdate {
            start: start,
            end: end,
            new_len: new_len,
            text: text,
            rev: rev,
            edit_type: edit_type,
            author: author
        }
    }
}

/// An simple edit, received from a plugin.
#[derive(Serialize, Deserialize, Debug)]
pub struct PluginEdit {
    pub start: u64,
    pub end: u64,
    pub rev: u64,
    pub text: String,
    /// the edit priority determines the resolution strategy when merging
    /// concurrent edits. The highest priority edit will be applied last.
    pub priority: u64,
    /// whether the inserted text prefers to be to the right of the cursor.
    pub after_cursor: bool,
    /// the originator of this edit: some identifier (plugin name, 'core', etc)
    pub author: String,
}

/// A response to an `update` RPC sent to a plugin.
#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum UpdateResponse {
    /// An edit to the buffer.
    Edit(PluginEdit),
    /// An acknowledgement with no action. A response cannot be Null, so we send a uint.
    Ack(u64),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub fg: u32,
    #[serde(rename = "font")]
    pub font_style: u8,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
/// RPC commands sent from the plugins.
pub enum PluginCommand {
    SetFgSpans {start: usize, len: usize, spans: Vec<Span>, rev: usize },
    GetData { offset: usize, max_size: usize, rev: usize },
    Alert { msg: String },
    LineCount,
}
