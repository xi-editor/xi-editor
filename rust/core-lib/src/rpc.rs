// Copyright 2016 Google Inc. All rights reserved.
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

//! RPC handling for communications with front-end.

use serde_json::{self, Value};
use serde::de::{self, Deserialize, Deserializer};
use serde::ser::{self, Serialize, Serializer};

use tabs::ViewIdentifier;
use plugins::PlaceholderRpc;


// =============================================================================
//  Command types
// =============================================================================

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct EmptyStruct {}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
pub enum CoreNotification {
    Edit(EditCommand<EditNotification>),
    Plugin(PluginNotification),
    CloseView { view_id: ViewIdentifier },
    Save { view_id: ViewIdentifier, file_path: String },
    SetTheme { theme_name: String },
    ClientStarted(EmptyStruct),
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
pub enum CoreRequest {
    Edit(EditCommand<EditRequest>),
    NewView { file_path: Option<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct EditCommand<T> {
    pub view_id: ViewIdentifier,
    pub cmd: T,
}

/// An enum representing touch and mouse gestures applied to the text.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum GestureType {
    ToggleSel,
}

// NOTE:
// Several core protocol commands use a params array to pass arguments
// which are named, internally. these two types use custom Serialize /
// Deserialize impls to accomodate this.
#[derive(PartialEq, Eq, Debug)]
pub struct LineRange {
    pub first: i64,
    pub last: i64,
}

#[derive(PartialEq, Eq, Debug)]
pub struct MouseAction {
    pub line: u64,
    pub column: u64,
    pub flags: u64,
    pub click_count: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
pub enum EditNotification {
    Insert { chars: String },
    DeleteForward,
    DeleteBackward,
    DeleteWordForward,
    DeleteWordBackward,
    DeleteToEndOfParagraph,
    DeleteToBeginningOfLine,
    InsertNewline,
    InsertTab,
    MoveUp,
    MoveUpAndModifySelection,
    MoveDown,
    MoveDownAndModifySelection,
    MoveLeft,
    // synoynm for `MoveLeft`
    MoveBackward,
    MoveLeftAndModifySelection,
    MoveRight,
    // synoynm for `MoveRight`
    MoveForward,
    MoveRightAndModifySelection,
    MoveWordLeft,
    MoveWordLeftAndModifySelection,
    MoveWordRight,
    MoveWordRightAndModifySelection,
    MoveToBeginningOfParagraph,
    MoveToEndOfParagraph,
    MoveToLeftEndOfLine,
    MoveToLeftEndOfLineAndModifySelection,
    MoveToRightEndOfLine,
    MoveToRightEndOfLineAndModifySelection,
    MoveToBeginningOfDocument,
    MoveToBeginningOfDocumentAndModifySelection,
    MoveToEndOfDocument,
    MoveToEndOfDocumentAndModifySelection,
    ScrollPageUp,
    PageUpAndModifySelection,
    ScrollPageDown,
    PageDownAndModifySelection,
    SelectAll,
    AddSelectionAbove,
    AddSelectionBelow,
    Scroll(LineRange),
    GotoLine { line: u64 },
    RequestLines(LineRange),
    Yank,
    Transpose,
    Click(MouseAction),
    Drag(MouseAction),
    Gesture { line: u64, column: u64, ty: GestureType},
    Undo,
    Redo,
    FindNext { wrap_around: Option<bool>, allow_same: Option<bool> },
    FindPrevious { wrap_around: Option<bool> },
    DebugRewrap,
    DebugPrintSpans,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
pub enum EditRequest {
    Cut,
    Copy,
    Find { chars: Option<String>, case_sensitive: bool },
}


#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "command")]
#[serde(rename_all = "snake_case")]
pub enum PluginNotification {
    Start { view_id: ViewIdentifier, plugin_name: String },
    Stop { view_id: ViewIdentifier, plugin_name: String },
    PluginRpc { view_id: ViewIdentifier, receiver: String, rpc: PlaceholderRpc },
}

// Serialize / Deserialize

impl<T: Serialize> Serialize for EditCommand<T>
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where S: Serializer
    {
        let mut v = serde_json::to_value(&self.cmd).map_err(ser::Error::custom)?;
        v["params"]["view_id"] = json!(self.view_id);
        v.serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for EditCommand<T>
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where D: Deserializer<'de>
    {
        #[derive(Deserialize)]
        struct InnerId {
            view_id: ViewIdentifier,
        }

        let mut v = Value::deserialize(deserializer)?;
        let helper = InnerId::deserialize(&v).map_err(de::Error::custom)?;
        let InnerId { view_id } = helper;
        // if params are empty, remove them
        let remove_params = match v.get("params") {
            Some(&Value::Object(ref obj)) => obj.is_empty(),
            Some(&Value::Array(ref arr)) => arr.is_empty(),
            Some(_) => return Err(de::Error::custom("'params' field, if present, must be object or array.")),
            None => false,
        };

        if remove_params {
            v.as_object_mut().map(|v| v.remove("params"));
        }

        let cmd = T::deserialize(v).map_err(de::Error::custom)?;
        Ok(EditCommand { view_id, cmd })
    }
}

impl Serialize for MouseAction
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where S: Serializer
    {
        #[derive(Serialize)]
        struct Helper(u64, u64, u64, Option<u64>);

        let as_tup = Helper(self.line, self.column, self.flags, self.click_count);
        let v = serde_json::to_value(&as_tup).map_err(ser::Error::custom)?;
        v.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for MouseAction
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where D: Deserializer<'de>
    {
        let v: Vec<u64> = Vec::deserialize(deserializer)?;
        let click_count = if v.len() == 4 { Some(v[3]) } else { None };
        Ok(MouseAction { line: v[0], column: v[1], flags: v[2], click_count: click_count })
    }
}

impl Serialize for LineRange
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where S: Serializer
    {
        let as_tup = (self.first, self.last);
        let v = serde_json::to_value(&as_tup).map_err(ser::Error::custom)?;
        v.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LineRange
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where D: Deserializer<'de>
    {
        #[derive(Deserialize)]
        struct TwoTuple(i64, i64);

        let tup = TwoTuple::deserialize(deserializer)?;
        Ok(LineRange { first: tup.0, last: tup.1 })
    }
}
