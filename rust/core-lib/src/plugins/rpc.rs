// Copyright 2017 The xi-editor Authors.
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

use std::borrow::Borrow;
use std::path::PathBuf;

use serde::de::{self, Deserialize, Deserializer};
use serde::ser::{self, Serialize, Serializer};
use serde_json::{self, Value};

use super::PluginPid;
use crate::annotations::AnnotationType;
use crate::config::Table;
use crate::syntax::LanguageId;
use crate::tabs::{BufferIdentifier, ViewId};
use xi_rope::{LinesMetric, Rope, RopeDelta};
use xi_rpc::RemoteError;

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
    pub views: Vec<ViewId>,
    pub rev: u64,
    pub buf_size: usize,
    pub nb_lines: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub syntax: LanguageId,
    pub config: Table,
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
    pub view_id: ViewId,
    /// The delta representing changes to the document.
    ///
    /// Note: Is `Some` in the general case; only if the delta involves
    /// inserting more than some maximum number of bytes, will this be `None`,
    /// indicating the plugin should flush cache and fetch manually.
    pub delta: Option<RopeDelta>,
    /// The size of the document after applying this delta.
    pub new_len: usize,
    /// The total number of lines in the document after applying this delta.
    pub new_line_count: usize,
    pub rev: u64,
    /// The undo_group associated with this update. The plugin may pass
    /// this value back to core when making an edit, to associate the
    /// plugin's edit with this undo group. Core uses undo_group
    //  to undo actions occurred due to plugins after a user action
    // in a single step.
    pub undo_group: Option<usize>,
    pub edit_type: String,
    pub author: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmptyStruct {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
/// RPC requests sent from the host
pub enum HostRequest {
    Update(PluginUpdate),
    CollectTrace(EmptyStruct),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
/// RPC Notifications sent from the host
pub enum HostNotification {
    Ping(EmptyStruct),
    Initialize { plugin_id: PluginPid, buffer_info: Vec<PluginBufferInfo> },
    DidSave { view_id: ViewId, path: PathBuf },
    ConfigChanged { view_id: ViewId, changes: Table },
    NewBuffer { buffer_info: Vec<PluginBufferInfo> },
    DidClose { view_id: ViewId },
    GetHover { view_id: ViewId, request_id: usize, position: usize },
    Shutdown(EmptyStruct),
    TracingConfig { enabled: bool },
    LanguageChanged { view_id: ViewId, new_lang: LanguageId },
    CustomCommand { view_id: ViewId, method: String, params: Value },
}

// ====================================================================
// plugin -> core RPC method types
// ====================================================================

/// A simple edit, received from a plugin.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PluginEdit {
    pub rev: u64,
    pub delta: RopeDelta,
    /// the edit priority determines the resolution strategy when merging
    /// concurrent edits. The highest priority edit will be applied last.
    pub priority: u64,
    /// whether the inserted text prefers to be to the right of the cursor.
    pub after_cursor: bool,
    /// the originator of this edit: some identifier (plugin name, 'core', etc)
    /// undo_group associated with this edit
    pub undo_group: Option<usize>,
    pub author: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct ScopeSpan {
    pub start: usize,
    pub end: usize,
    pub scope_id: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DataSpan {
    pub start: usize,
    pub end: usize,
    pub data: Value,
}

/// The object returned by the `get_data` RPC.
#[derive(Debug, Serialize, Deserialize)]
pub struct GetDataResponse {
    pub chunk: String,
    pub offset: usize,
    pub first_line: usize,
    pub first_line_offset: usize,
}

/// The unit of measure when requesting data.
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum TextUnit {
    /// The requested offset is in bytes. The returned chunk will be valid
    /// UTF8, and is guaranteed to include the byte specified the offset.
    Utf8,
    /// The requested offset is a line number. The returned chunk will begin
    /// at the offset of the requested line.
    Line,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
/// RPC requests sent from plugins.
pub enum PluginRequest {
    GetData { start: usize, unit: TextUnit, max_size: usize, rev: u64 },
    LineCount,
    GetSelections,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
/// RPC commands sent from plugins.
pub enum PluginNotification {
    AddScopes {
        scopes: Vec<Vec<String>>,
    },
    UpdateSpans {
        start: usize,
        len: usize,
        spans: Vec<ScopeSpan>,
        rev: u64,
    },
    Edit {
        edit: PluginEdit,
    },
    Alert {
        msg: String,
    },
    AddStatusItem {
        key: String,
        value: String,
        alignment: String,
    },
    UpdateStatusItem {
        key: String,
        value: String,
    },
    RemoveStatusItem {
        key: String,
    },
    ShowHover {
        request_id: usize,
        result: Result<Hover, RemoteError>,
    },
    UpdateAnnotations {
        start: usize,
        len: usize,
        spans: Vec<DataSpan>,
        annotation_type: AnnotationType,
        rev: u64,
    },
}

/// Range expressed in terms of PluginPosition. Meant to be sent from
/// plugin to core.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub struct Range {
    pub start: usize,
    pub end: usize,
}

/// Hover Item sent from Plugin to Core
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub struct Hover {
    pub content: String,
    pub range: Option<Range>,
}

/// Common wrapper for plugin-originating RPCs.
pub struct PluginCommand<T> {
    pub view_id: ViewId,
    pub plugin_id: PluginPid,
    pub cmd: T,
}

impl<T: Serialize> Serialize for PluginCommand<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut v = serde_json::to_value(&self.cmd).map_err(ser::Error::custom)?;
        v["params"]["view_id"] = json!(self.view_id);
        v["params"]["plugin_id"] = json!(self.plugin_id);
        v.serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for PluginCommand<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct InnerIds {
            view_id: ViewId,
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
    pub fn new(
        buffer_id: BufferIdentifier,
        views: &[ViewId],
        rev: u64,
        buf_size: usize,
        nb_lines: usize,
        path: Option<PathBuf>,
        syntax: LanguageId,
        config: Table,
    ) -> Self {
        //TODO: do make any current assertions about paths being valid utf-8? do we want to?
        let path = path.map(|p| p.to_str().unwrap().to_owned());
        let views = views.to_owned();
        PluginBufferInfo { buffer_id, views, rev, buf_size, nb_lines, path, syntax, config }
    }
}

impl PluginUpdate {
    pub fn new<D>(
        view_id: ViewId,
        rev: u64,
        delta: D,
        new_len: usize,
        new_line_count: usize,
        undo_group: Option<usize>,
        edit_type: String,
        author: String,
    ) -> Self
    where
        D: Into<Option<RopeDelta>>,
    {
        let delta = delta.into();
        PluginUpdate { view_id, delta, new_len, new_line_count, rev, undo_group, edit_type, author }
    }
}

// maybe this should be in xi_rope? has a strong resemblance to the various
// concrete `Metric` types.
impl TextUnit {
    /// Converts an offset in some unit to a concrete byte offset. Returns
    /// `None` if the input offset is out of bounds in its unit space.
    pub fn resolve_offset<T: Borrow<Rope>>(self, text: T, offset: usize) -> Option<usize> {
        let text = text.borrow();
        match self {
            TextUnit::Utf8 => {
                if offset > text.len() {
                    None
                } else {
                    text.at_or_prev_codepoint_boundary(offset)
                }
            }
            TextUnit::Line => {
                let max_line_number = text.measure::<LinesMetric>() + 1;
                if offset > max_line_number {
                    None
                } else {
                    text.offset_of_line(offset).into()
                }
            }
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
            "delta": {"base_len": 6, "els": [{"copy": [0,5]}, {"insert":"rofls"}, {"copy": [5,6]}]},
            "new_len": 11,
            "new_line_count": 1,
            "rev": 5,
            "undo_group": 6,
            "edit_type": "something",
            "author": "me"
    }"#;

        let val: PluginUpdate = match serde_json::from_str(json) {
            Ok(val) => val,
            Err(err) => panic!("{:?}", err),
        };
        assert!(val.delta.is_some());
        assert!(val.delta.unwrap().as_simple_insert().is_some());
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
             "syntax": "toml",
             "config": {"some_key": 420}}"#;

        let val: PluginBufferInfo = match serde_json::from_str(json) {
            Ok(val) => val,
            Err(err) => panic!("{:?}", err),
        };
        assert_eq!(val.rev, 1);
        assert_eq!(val.path, Some("some_path".to_owned()));
        assert_eq!(val.syntax, "toml".into());
    }

    #[test]
    fn test_de_plugin_rpc() {
        let json = r#"{"method": "alert", "params": {"view_id": "view-id-1", "plugin_id": 42, "msg": "ahhh!"}}"#;
        let de: PluginCommand<PluginNotification> = serde_json::from_str(json).unwrap();
        assert_eq!(de.view_id, ViewId(1));
        assert_eq!(de.plugin_id, PluginPid(42));
        match de.cmd {
            PluginNotification::Alert { ref msg } if msg == "ahhh!" => (),
            _ => panic!("{:?}", de.cmd),
        }
    }
}
