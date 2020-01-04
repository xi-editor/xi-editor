// Copyright 2016 The xi-editor Authors.
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

//! The main RPC protocol, for communication between `xi-core` and the client.
//!
//! We rely on [Serde] for serialization and deserialization between
//! the JSON-RPC protocol and the types here.
//!
//! [Serde]: https://serde.rs

use std::path::PathBuf;

use serde::de::{self, Deserialize, Deserializer};
use serde::ser::{self, Serialize, Serializer};
use serde_json::{self, Value};

use crate::config::{ConfigDomainExternal, Table};
use crate::plugins::PlaceholderRpc;
use crate::syntax::LanguageId;
use crate::tabs::ViewId;
use crate::view::Size;

// =============================================================================
//  Command types
// =============================================================================

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[doc(hidden)]
pub struct EmptyStruct {}

/// The notifications which make up the base of the protocol.
///
/// # Note
///
/// For serialization, all identifiers are converted to "snake_case".
///
/// # Examples
///
/// The `close_view` command:
///
/// ```
/// # extern crate xi_core_lib as xi_core;
/// extern crate serde_json;
/// use crate::xi_core::rpc::CoreNotification;
///
/// let json = r#"{
///     "method": "close_view",
///     "params": { "view_id": "view-id-1" }
///     }"#;
///
/// let cmd: CoreNotification = serde_json::from_str(&json).unwrap();
/// match cmd {
///     CoreNotification::CloseView { .. } => (), // expected
///     other => panic!("Unexpected variant"),
/// }
/// ```
///
/// The `client_started` command:
///
/// ```
/// # extern crate xi_core_lib as xi_core;
/// extern crate serde_json;
/// use crate::xi_core::rpc::CoreNotification;
///
/// let json = r#"{
///     "method": "client_started",
///     "params": {}
///     }"#;
///
/// let cmd: CoreNotification = serde_json::from_str(&json).unwrap();
/// match cmd {
///     CoreNotification::ClientStarted { .. }  => (), // expected
///     other => panic!("Unexpected variant"),
/// }
/// ```
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
pub enum CoreNotification {
    /// The 'edit' namespace, for view-specific editor actions.
    ///
    /// The params object has internal `method` and `params` members,
    /// which are parsed into the appropriate `EditNotification`.
    ///
    /// # Note:
    ///
    /// All edit commands (notifications and requests) include in their
    /// inner params object a `view_id` field. On the xi-core side, we
    /// pull out this value during parsing, and use it for routing.
    ///
    /// For more on the edit commands, see [`EditNotification`] and
    /// [`EditRequest`].
    ///
    /// [`EditNotification`]: enum.EditNotification.html
    /// [`EditRequest`]: enum.EditRequest.html
    ///
    /// # Examples
    ///
    /// ```
    /// # extern crate xi_core_lib as xi_core;
    /// #[macro_use]
    /// extern crate serde_json;
    /// use crate::xi_core::rpc::*;
    /// # fn main() {
    /// let edit = EditCommand {
    ///     view_id: 1.into(),
    ///     cmd: EditNotification::Insert { chars: "hello!".into() },
    /// };
    /// let rpc = CoreNotification::Edit(edit);
    /// let expected = json!({
    ///     "method": "edit",
    ///     "params": {
    ///         "method": "insert",
    ///         "view_id": "view-id-1",
    ///         "params": {
    ///             "chars": "hello!",
    ///         }
    ///     }
    /// });
    /// assert_eq!(serde_json::to_value(&rpc).unwrap(), expected);
    /// # }
    /// ```
    Edit(EditCommand<EditNotification>),
    /// The 'plugin' namespace, for interacting with plugins.
    ///
    /// As with edit commands, the params object has is a nested RPC,
    /// with the name of the command included as the `command` field.
    ///
    /// (this should be changed to more accurately reflect the behaviour
    /// of the edit commands).
    ///
    /// For the available commands, see [`PluginNotification`].
    ///
    /// [`PluginNotification`]: enum.PluginNotification.html
    ///
    /// # Examples
    ///
    /// ```
    /// # extern crate xi_core_lib as xi_core;
    /// #[macro_use]
    /// extern crate serde_json;
    /// use crate::xi_core::rpc::*;
    /// # fn main() {
    /// let rpc = CoreNotification::Plugin(
    ///     PluginNotification::Start {
    ///         view_id: 1.into(),
    ///         plugin_name: "syntect".into(),
    ///     });
    ///
    /// let expected = json!({
    ///     "method": "plugin",
    ///     "params": {
    ///         "command": "start",
    ///         "view_id": "view-id-1",
    ///         "plugin_name": "syntect",
    ///     }
    /// });
    /// assert_eq!(serde_json::to_value(&rpc).unwrap(), expected);
    /// # }
    /// ```
    Plugin(PluginNotification),
    /// Tells `xi-core` to close the specified view.
    CloseView { view_id: ViewId },
    /// Tells `xi-core` to save the contents of the specified view's
    /// buffer to the specified path.
    Save { view_id: ViewId, file_path: String },
    /// Tells `xi-core` to set the theme.
    SetTheme { theme_name: String },
    /// Notifies `xi-core` that the client has started.
    ClientStarted {
        #[serde(default)]
        config_dir: Option<PathBuf>,
        /// Path to additional plugins, included by the client.
        #[serde(default)]
        client_extras_dir: Option<PathBuf>,
    },
    /// Updates the user's config for the given domain. Where keys in
    /// `changes` are `null`, those keys are cleared in the user config
    /// for that domain; otherwise the config is updated with the new
    /// value.
    ///
    /// Note: If the client is using file-based config, the only valid
    /// domain argument is `ConfigDomain::UserOverride(_)`, which
    /// represents non-persistent view-specific settings, such as when
    /// a user manually changes whitespace settings for a given view.
    ModifyUserConfig { domain: ConfigDomainExternal, changes: Table },
    /// Control whether the tracing infrastructure is enabled.
    /// This propagates to all peers that should respond by toggling its own
    /// infrastructure on/off.
    TracingConfig { enabled: bool },
    /// Save trace data to the given path.  The core will first send
    /// CoreRequest::CollectTrace to all peers to collect the samples.
    SaveTrace { destination: PathBuf, frontend_samples: Value },
    /// Tells `xi-core` to set the language id for the view.
    SetLanguage { view_id: ViewId, language_id: LanguageId },
}

/// The requests which make up the base of the protocol.
///
/// All requests expect a response.
///
/// # Examples
///
/// The `new_view` command:
///
/// ```
/// # extern crate xi_core_lib as xi_core;
/// extern crate serde_json;
/// use crate::xi_core::rpc::CoreRequest;
///
/// let json = r#"{
///     "method": "new_view",
///     "params": { "file_path": "~/my_very_fun_file.rs" }
///     }"#;
///
/// let cmd: CoreRequest = serde_json::from_str(&json).unwrap();
/// match cmd {
///     CoreRequest::NewView { .. } => (), // expected
///     other => panic!("Unexpected variant {:?}", other),
/// }
/// ```
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
pub enum CoreRequest {
    /// The 'edit' namespace, for view-specific requests.
    Edit(EditCommand<EditRequest>),
    /// Tells `xi-core` to create a new view. If the `file_path`
    /// argument is present, `xi-core` should attempt to open the file
    /// at that location.
    ///
    /// Returns the view identifier that should be used to interact
    /// with the newly created view.
    NewView { file_path: Option<String> },
    /// Returns the current collated config object for the given view.
    GetConfig { view_id: ViewId },
    /// Returns the contents of the buffer for a given `ViewId`.
    /// In the future this might also be used to return structured data (such
    /// as for printing).
    DebugGetContents { view_id: ViewId },
}

/// A helper type, which extracts the `view_id` field from edit
/// requests and notifications.
///
/// Edit requests and notifications have 'method', 'params', and
/// 'view_id' param members. We use this wrapper, which has custom
/// `Deserialize` and `Serialize` implementations, to pull out the
/// `view_id` field.
///
/// # Examples
///
/// ```
/// # extern crate xi_core_lib as xi_core;
/// extern crate serde_json;
/// use crate::xi_core::rpc::*;
///
/// let json = r#"{
///     "view_id": "view-id-1",
///     "method": "scroll",
///     "params": [0, 6]
///     }"#;
///
/// let cmd: EditCommand<EditNotification> = serde_json::from_str(&json).unwrap();
/// match cmd.cmd {
///     EditNotification::Scroll( .. ) => (), // expected
///     other => panic!("Unexpected variant {:?}", other),
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct EditCommand<T> {
    pub view_id: ViewId,
    pub cmd: T,
}

/// The smallest unit of text that a gesture can select
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Copy, Clone)]
#[serde(rename_all = "snake_case")]
pub enum SelectionGranularity {
    /// Selects any point or character range
    Point,
    /// Selects one word at a time
    Word,
    /// Selects one line at a time
    Line,
}

/// An enum representing touch and mouse gestures applied to the text.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Copy, Clone)]
#[serde(rename_all = "snake_case")]
pub enum GestureType {
    Select { granularity: SelectionGranularity, multi: bool },
    SelectExtend { granularity: SelectionGranularity },
    Drag,

    // Deprecated
    PointSelect,
    ToggleSel,
    RangeSelect,
    LineSelect,
    WordSelect,
    MultiLineSelect,
    MultiWordSelect,
}

/// An inclusive range.
///
/// # Note:
///
/// Several core protocol commands use a params array to pass arguments
/// which are named, internally. this type use custom Serialize /
/// Deserialize impls to accommodate this.
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct LineRange {
    pub first: i64,
    pub last: i64,
}

/// A mouse event. See the note for [`LineRange`].
///
/// [`LineRange`]: enum.LineRange.html
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct MouseAction {
    pub line: u64,
    pub column: u64,
    pub flags: u64,
    pub click_count: Option<u64>,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

/// Represents how the current selection is modified (used by find
/// operations).
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum SelectionModifier {
    None,
    Set,
    Add,
    AddRemovingCurrent,
}

impl Default for SelectionModifier {
    fn default() -> SelectionModifier {
        SelectionModifier::Set
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub struct FindQuery {
    pub id: Option<usize>,
    pub chars: String,
    pub case_sensitive: bool,
    #[serde(default)]
    pub regex: bool,
    #[serde(default)]
    pub whole_words: bool,
}

/// The edit-related notifications.
///
/// Alongside the [`EditRequest`] members, these commands constitute
/// the API for interacting with a particular window and document.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
pub enum EditNotification {
    Insert {
        chars: String,
    },
    Paste {
        chars: String,
    },
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
    MoveToBeginningOfParagraphAndModifySelection,
    MoveToEndOfParagraph,
    MoveToEndOfParagraphAndModifySelection,
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
    Resize(Size),
    GotoLine {
        line: u64,
    },
    RequestLines(LineRange),
    Yank,
    Transpose,
    Click(MouseAction),
    Drag(MouseAction),
    Gesture {
        line: u64,
        col: u64,
        ty: GestureType,
    },
    Undo,
    Redo,
    Find {
        chars: String,
        case_sensitive: bool,
        #[serde(default)]
        regex: bool,
        #[serde(default)]
        whole_words: bool,
    },
    MultiFind {
        queries: Vec<FindQuery>,
    },
    FindNext {
        #[serde(default)]
        wrap_around: bool,
        #[serde(default)]
        allow_same: bool,
        #[serde(default)]
        modify_selection: SelectionModifier,
    },
    FindPrevious {
        #[serde(default)]
        wrap_around: bool,
        #[serde(default)]
        allow_same: bool,
        #[serde(default)]
        modify_selection: SelectionModifier,
    },
    FindAll,
    DebugRewrap,
    DebugWrapWidth,
    /// Prints the style spans present in the active selection.
    DebugPrintSpans,
    DebugToggleComment,
    Uppercase,
    Lowercase,
    Capitalize,
    Reindent,
    Indent,
    Outdent,
    /// Indicates whether find highlights should be rendered
    HighlightFind {
        visible: bool,
    },
    SelectionForFind {
        #[serde(default)]
        case_sensitive: bool,
    },
    Replace {
        chars: String,
        #[serde(default)]
        preserve_case: bool,
    },
    ReplaceNext,
    ReplaceAll,
    SelectionForReplace,
    RequestHover {
        request_id: usize,
        position: Option<Position>,
    },
    SelectionIntoLines,
    DuplicateLine,
    IncreaseNumber,
    DecreaseNumber,
    ToggleRecording {
        recording_name: Option<String>,
    },
    PlayRecording {
        recording_name: String,
    },
    ClearRecording {
        recording_name: String,
    },
    CollapseSelections,
}

/// The edit related requests.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
pub enum EditRequest {
    /// Cuts the active selection, returning their contents,
    /// or `Null` if the selection was empty.
    Cut,
    /// Copies the active selection, returning their contents or
    /// or `Null` if the selection was empty.
    Copy,
}

/// The plugin related notifications.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "command")]
#[serde(rename_all = "snake_case")]
pub enum PluginNotification {
    Start { view_id: ViewId, plugin_name: String },
    Stop { view_id: ViewId, plugin_name: String },
    PluginRpc { view_id: ViewId, receiver: String, rpc: PlaceholderRpc },
}

// Serialize / Deserialize

impl<T: Serialize> Serialize for EditCommand<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut v = serde_json::to_value(&self.cmd).map_err(ser::Error::custom)?;
        v["view_id"] = json!(self.view_id);
        v.serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for EditCommand<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct InnerId {
            view_id: ViewId,
        }

        let mut v = Value::deserialize(deserializer)?;
        let helper = InnerId::deserialize(&v).map_err(de::Error::custom)?;
        let InnerId { view_id } = helper;

        // if params are empty, remove them
        let remove_params = match v.get("params") {
            Some(&Value::Object(ref obj)) => obj.is_empty() && T::deserialize(v.clone()).is_err(),
            Some(&Value::Array(ref arr)) => arr.is_empty() && T::deserialize(v.clone()).is_err(),
            Some(_) => {
                return Err(de::Error::custom(
                    "'params' field, if present, must be object or array.",
                ));
            }
            None => false,
        };

        if remove_params {
            v.as_object_mut().map(|v| v.remove("params"));
        }

        let cmd = T::deserialize(v).map_err(de::Error::custom)?;
        Ok(EditCommand { view_id, cmd })
    }
}

impl Serialize for MouseAction {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct Helper(u64, u64, u64, Option<u64>);

        let as_tup = Helper(self.line, self.column, self.flags, self.click_count);
        as_tup.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for MouseAction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v: Vec<u64> = Vec::deserialize(deserializer)?;
        let click_count = if v.len() == 4 { Some(v[3]) } else { None };
        Ok(MouseAction { line: v[0], column: v[1], flags: v[2], click_count })
    }
}

impl Serialize for LineRange {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let as_tup = (self.first, self.last);
        as_tup.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LineRange {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct TwoTuple(i64, i64);

        let tup = TwoTuple::deserialize(deserializer)?;
        Ok(LineRange { first: tup.0, last: tup.1 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tabs::ViewId;

    #[test]
    fn test_serialize_edit_command() {
        // Ensure that an EditCommand can be serialized and then correctly deserialized.
        let message: String = "hello world".into();
        let edit = EditCommand {
            view_id: ViewId(1),
            cmd: EditNotification::Insert { chars: message.clone() },
        };
        let json = serde_json::to_string(&edit).unwrap();
        let cmd: EditCommand<EditNotification> = serde_json::from_str(&json).unwrap();
        assert_eq!(cmd.view_id, edit.view_id);
        if let EditNotification::Insert { chars } = cmd.cmd {
            assert_eq!(chars, message);
        }
    }
}
