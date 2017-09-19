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

use std::path::PathBuf;

use serde::de::{self, Deserialize, Deserializer};
use serde::ser::{self, Serialize, Serializer};
use serde_json::{self, Value};

use super::PluginPid;
use syntax::SyntaxDefinition;
use tabs::{BufferIdentifier, ViewIdentifier};

//TODO: At the moment (May 08, 2017) this is all very much in flux.
// At some point, it will be stabalized and then perhaps will live in another crate,
// shared with the plugin lib.

// ====================================================================
// core -> plugin RPC method types + responses
// ====================================================================

/// Buffer information sent on plugin init.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PluginBufferInfo {
    /// The buffer's unique identifier.
    pub buffer_id: BufferIdentifier,
    /// The buffer's current views.
    pub views: Vec<ViewIdentifier>,
    pub rev: u64,
    pub buf_size: usize,
    pub nb_lines: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub syntax: SyntaxDefinition,
}

//TODO: very likely this should be merged with PluginDescription
//TODO: also this does not belong here.
/// Describes an available plugin to the client.
#[derive(Serialize, Deserialize, Debug)]
pub struct ClientPluginInfo {
    pub name: String,
    pub running: bool,
}

/// A simple update, sent to a plugin.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PluginUpdate {
    view_id: ViewIdentifier,
    start: usize,
    end: usize,
    new_len: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    rev: u64,
    edit_type: String,
    author: String,
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


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
/// RPC requests sent from the host
pub enum HostRequest {
    Update(PluginUpdate),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
/// RPC Notifications sent from the host
pub enum HostNotification {
    Ping,
    Initialize { plugin_id: PluginPid, buffer_info: Vec<PluginBufferInfo> },
    DidSave { view_id: ViewIdentifier, path: PathBuf },
    NewBuffer { buffer_info: Vec<PluginBufferInfo> },
    DidClose { view_id: ViewIdentifier },
    Shutdown,
}


// ====================================================================
// plugin -> core RPC method types
// ====================================================================


/// A simple edit, received from a plugin.
//TODO: use a real delta here
#[derive(Serialize, Deserialize, Debug, Clone)]
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

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct ScopeSpan {
    pub start: usize,
    pub end: usize,
    pub scope_id: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
/// RPC requests sent from plugins.
pub enum PluginRequest {
    GetData { offset: usize, max_size: usize, rev: u64 },
    LineCount,
    GetSelections,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
/// RPC commands sent from plugins.
pub enum PluginNotification {
    AddScopes { scopes: Vec<Vec<String>> },
    UpdateSpans { start: usize, len: usize, spans: Vec<ScopeSpan>, rev: u64 },
    Edit { edit: PluginEdit },
    Alert { msg: String },
}

/// Common wrapper for plugin-originating RPCs.
pub struct PluginCommand<T> {
    pub view_id: ViewIdentifier,
    pub plugin_id: PluginPid,
    pub cmd: T,
}

impl<T: Serialize> Serialize for PluginCommand<T>
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where S: Serializer
    {
        let mut v = serde_json::to_value(&self.cmd).map_err(ser::Error::custom)?;
        v["params"]["view_id"] = json!(self.view_id);
        v["params"]["plugin_id"] = json!(self.plugin_id);
        v.serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for PluginCommand<T>
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where D: Deserializer<'de>
    {
        #[derive(Deserialize)]
        struct InnerIds {
            view_id: ViewIdentifier,
            plugin_id: PluginPid,
        }
        #[derive(Deserialize)]
        struct IdsWrapper {
            params: InnerIds,
        }

        let v = Value::deserialize(deserializer)?;
        let helper = IdsWrapper::deserialize(&v).map_err(de::Error::custom)?;
        let InnerIds { view_id, plugin_id } = helper.params;
        let cmd = T::deserialize(v).map_err(de::Error::custom)?;
        Ok(PluginCommand { view_id, plugin_id, cmd })
    }
}

impl PluginBufferInfo {
    pub fn new(buffer_id: BufferIdentifier, views: &[ViewIdentifier],
               rev: u64, buf_size: usize, nb_lines: usize,
               path: Option<PathBuf>, syntax: SyntaxDefinition) -> Self {
        //TODO: do make any current assertions about paths being valid utf-8? do we want to?
        let path = path.map(|p| p.to_str().unwrap().to_owned());
        let views = views.to_owned();
        PluginBufferInfo { buffer_id, views, rev, buf_size, nb_lines, path, syntax }
    }
}

impl PluginUpdate {
    pub fn new(view_id: ViewIdentifier, start: usize, end: usize,
               new_len: usize, rev: u64, text: Option<String>,
               edit_type: String, author: String) -> Self {
        PluginUpdate {
            view_id: view_id,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_plugin_update() {
        let json = r#"{
            "view_id": "view-id-42",
            "start": 1,
            "end": 5,
            "new_len": 2,
            "rev": 5,
            "edit_type": "something",
            "author": "me"
    }"#;

    let val: PluginUpdate = match serde_json::from_str(json) {
        Ok(val) => val,
        Err(err) => panic!("{:?}", err),
    };
    assert!(val.text.is_none());
    assert_eq!(val.start, 1);
    }

    #[test]
    fn test_deserde_init() {
        let json = r#"
            {"buffer_id": 42,
             "views": ["view-id-4"],
             "rev": 1,
             "buf_size": 20,
             "nb_lines": 5,
             "path": "some_path",
             "syntax": "toml"}"#;

        let val: PluginBufferInfo = match serde_json::from_str(json) {
            Ok(val) => val,
            Err(err) => panic!("{:?}", err),
        };
        assert_eq!(val.rev, 1);
        assert_eq!(val.path, Some("some_path".to_owned()));
        assert_eq!(val.syntax, SyntaxDefinition::Toml);
    }

    #[test]
    fn test_de_plugin_rpc() {
        let json = r#"{"method": "alert", "params": {"view_id": "view-id-1", "plugin_id": 42, "msg": "ahhh!"}}"#;
        let de: PluginNotificationRpc = serde_json::from_str(json).unwrap();
        assert_eq!(de.view_id, "view-id-1".into());
        assert_eq!(de.plugin_id, PluginPid(42));
        match de.cmd {
            PluginNotification::Alert { ref msg } if msg == "ahhh!" => (),
            _ => panic!("{:?}", de.cmd),
        }
    }
}
