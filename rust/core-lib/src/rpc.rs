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

use std::error;
use std::fmt;
use serde_json::Value;
use xi_rpc::{dict_get_u64, dict_get_string, arr_get_u64, arr_get_i64};

// =============================================================================
//  Request handling
// =============================================================================

impl<'a> Request<'a> {
    pub fn from_json(method: &'a str, params: &'a Value) -> Result<Self, Error> {
        CoreCommand::from_json(method, params).map(|cmd|
            Request::CoreCommand { core_command: cmd})
    }
}

// =============================================================================
//  Command types
// =============================================================================

#[derive(Debug, PartialEq)]
pub enum Request<'a> {
    CoreCommand { core_command: CoreCommand<'a> }
}

/// An enum representing a core command, parsed from JSON.
#[derive(Debug, PartialEq, Eq)]
pub enum CoreCommand<'a> {
    Edit { view_id: &'a str, edit_command: EditCommand<'a> },
    /// Request a new view, opening a file if `file_path` is Some, else creating an empty buffer.
    NewView { file_path: Option<&'a str> },
    CloseView { view_id: &'a str },
    Save { view_id: &'a str, file_path: &'a str },
}

/// An enum representing an edit command, parsed from JSON.
#[derive(Debug, PartialEq, Eq)]
pub enum EditCommand<'a> {
    Key { chars: &'a str, flags: u64 },
    Insert { chars: &'a str },
    DeleteForward,
    DeleteBackward,
    DeleteToEndOfParagraph,
    DeleteToBeginningOfLine,
    InsertNewline,
    InsertTab,
    MoveUp,
    MoveUpAndModifySelection,
    MoveDown,
    MoveDownAndModifySelection,
    MoveLeft,
    MoveLeftAndModifySelection,
    MoveRight,
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
    Scroll { first: i64, last: i64 },
    GotoLine { line: u64 },
    RequestLines { first: i64, last: i64 },
    Yank,
    Transpose,
    Click { line: u64, column: u64, flags: u64, click_count: u64 },
    Drag { line: u64, column: u64, flags: u64 },
    Undo,
    Redo,
    Cut,
    Copy,
    DebugRewrap,
    DebugTestFgSpans,
    DebugRunPlugin,
    DebugTestSoftSpans,
}

impl<'a> CoreCommand<'a> {
    pub fn from_json(method: &str, params: &'a Value) -> Result<Self, Error> {
        use self::CoreCommand::*;
        use self::Error::*;

        match method {
            "close_view" => params.as_object().and_then(|dict| {
                dict_get_string(dict, "view_id").map(|view_id| CloseView { view_id: view_id })
            }).ok_or_else(|| MalformedCoreParams(method.to_string(), params.clone())),

            "new_view" => params.as_object()
                .map(|dict| NewView { file_path: dict_get_string(dict, "file_path") }) // optional
                .ok_or_else(|| MalformedEditParams(method.to_string(), params.clone())),
                
            "save" => params.as_object().and_then(|dict| {
                dict_get_string(dict, "view_id").and_then(|view_id| {
                    dict_get_string(dict, "file_path").map(|file_path| {
                        Save { view_id: view_id, file_path: file_path }
                       })
                    })
                }).ok_or_else(|| MalformedCoreParams(method.to_string(), params.clone())),

            "edit" => params.as_object()
                .ok_or_else(|| MalformedCoreParams(method.to_string(), params.clone()))
                .and_then(|dict| {
                    if let (Some(view_id), Some(method), Some(edit_params)) =
                        (dict_get_string(dict, "view_id"), dict_get_string(dict, "method"), dict.get("params")) {
                            EditCommand::from_json(method, edit_params)
                                .map(|cmd| Edit { view_id: view_id, edit_command: cmd })
                        } else { Err(MalformedCoreParams(method.to_string(), params.clone())) }
                }),

            _ => Err(UnknownCoreMethod(method.to_string()))
        }
    }
}

impl<'a> EditCommand<'a> {
    /// Try to read an edit command with the given method and parameters.
    pub fn from_json(method: &str, params: &'a Value) -> Result<Self, Error> {
        use self::EditCommand::*;
        use self::Error::*;

        match method {
            "key" => params.as_object().and_then(|dict| {
                dict_get_string(dict, "chars").and_then(|chars| {
                    dict_get_u64(dict, "flags").map(|flags| {
                        Key { chars: chars, flags: flags }
                    })
                })
            }).ok_or_else(|| MalformedEditParams(method.to_string(), params.clone())),

            "insert" => params.as_object().and_then(|dict| {
                dict_get_string(dict, "chars").map(|chars| Insert { chars: chars })
            }).ok_or_else(|| MalformedEditParams(method.to_string(), params.clone())),

            "delete_forward" => Ok(DeleteForward),
            "delete_backward" => Ok(DeleteBackward),
            "delete_to_end_of_paragraph" => Ok(DeleteToEndOfParagraph),
            "delete_to_beginning_of_line" => Ok(DeleteToBeginningOfLine),
            "insert_newline" => Ok(InsertNewline),
            "insert_tab" => Ok(InsertTab),
            "move_up" => Ok(MoveUp),
            "move_up_and_modify_selection" => Ok(MoveUpAndModifySelection),
            "move_down" => Ok(MoveDown),
            "move_down_and_modify_selection" => Ok(MoveDownAndModifySelection),
            "move_left" | "move_backward" => Ok(MoveLeft),
            "move_left_and_modify_selection" => Ok(MoveLeftAndModifySelection),
            "move_word_left" => Ok(MoveWordLeft),
            "move_word_left_and_modify_selection" => Ok(MoveWordLeftAndModifySelection),
            "move_word_right" => Ok(MoveWordRight),
            "move_word_right_and_modify_selection" => Ok(MoveWordRightAndModifySelection),
            "move_right" | "move_forward" => Ok(MoveRight),
            "move_right_and_modify_selection" => Ok(MoveRightAndModifySelection),
            "move_to_beginning_of_paragraph" => Ok(MoveToBeginningOfParagraph),
            "move_to_end_of_paragraph" => Ok(MoveToEndOfParagraph),
            "move_to_left_end_of_line" => Ok(MoveToLeftEndOfLine),
            "move_to_left_end_of_line_and_modify_selection" => Ok(MoveToLeftEndOfLineAndModifySelection),
            "move_to_right_end_of_line" => Ok(MoveToRightEndOfLine),
            "move_to_right_end_of_line_and_modify_selection" => Ok(MoveToRightEndOfLineAndModifySelection),
            "move_to_beginning_of_document" => Ok(MoveToBeginningOfDocument),
            "move_to_beginning_of_document_and_modify_selection" => Ok(MoveToBeginningOfDocumentAndModifySelection),
            "move_to_end_of_document" => Ok(MoveToEndOfDocument),
            "move_to_end_of_document_and_modify_selection" => Ok(MoveToEndOfDocumentAndModifySelection),
            "scroll_page_up" | "page_up" => Ok(ScrollPageUp),
            "page_up_and_modify_selection" => Ok(PageUpAndModifySelection),
            "scroll_page_down" |
            "page_down" => Ok(ScrollPageDown),
            "page_down_and_modify_selection" => Ok(PageDownAndModifySelection),
            "select_all" => Ok(SelectAll),

            "scroll" => params.as_array().and_then(|arr| {
                if let (Some(first), Some(last)) =
                    (arr_get_i64(arr, 0), arr_get_i64(arr, 1)) {

                    Some(Scroll { first: first, last: last })
                } else { None }
            }).ok_or_else(|| MalformedEditParams(method.to_string(), params.clone())),

            "goto_line" => params.as_object().and_then(|dict| {
                dict_get_u64(dict, "line").map(|line| GotoLine { line: line })
            }).ok_or_else(|| MalformedEditParams(method.to_string(), params.clone())),

            "request_lines" => params.as_array().and_then(|arr| {
                if let (Some(first), Some(last)) =
                    (arr_get_i64(arr, 0), arr_get_i64(arr, 1)) {

                    Some(RequestLines { first: first, last: last })
                } else { None }
            }).ok_or_else(|| MalformedEditParams(method.to_string(), params.clone())),

            "yank" => Ok(Yank),
            "transpose" => Ok(Transpose),

            "click" => params.as_array().and_then(|arr| {
                if let (Some(line), Some(column), Some(flags), Some(click_count)) =
                    (arr_get_u64(arr, 0), arr_get_u64(arr, 1), arr_get_u64(arr, 2), arr_get_u64(arr, 3)) {

                        Some(Click { line: line, column: column, flags: flags, click_count: click_count })
                    } else { None }
            }).ok_or_else(|| MalformedEditParams(method.to_string(), params.clone())),

            "drag" => params.as_array().and_then(|arr| {
                if let (Some(line), Some(column), Some(flags)) =
                    (arr_get_u64(arr, 0), arr_get_u64(arr, 1), arr_get_u64(arr, 2)) {

                        Some(Drag { line: line, column: column, flags: flags })
                    } else { None }
            }).ok_or_else(|| MalformedEditParams(method.to_string(), params.clone())),

            "undo" => Ok(Undo),
            "redo" => Ok(Redo),
            "cut" => Ok(Cut),
            "copy" => Ok(Copy),
            "debug_rewrap" => Ok(DebugRewrap),
            "debug_test_fg_spans" => Ok(DebugTestFgSpans),
            "debug_run_plugin" => Ok(DebugRunPlugin),
            "debug_test_soft_spans" => Ok(DebugTestSoftSpans),

            _ => Err(UnknownEditMethod(method.to_string())),
        }
    }
}

// =============================================================================
//  Error types
// =============================================================================

/// An error that occurred while parsing an edit command.
#[derive(Debug, PartialEq)]
pub enum Error {
    UnknownCoreMethod(String), // method name
    MalformedCoreParams(String, Value), // method name, malformed params
    UnknownEditMethod(String), // method name
    MalformedEditParams(String, Value), // method name, malformed params
}

impl fmt::Display for Error {
    // TODO: Provide information about the parameter format expected when
    // displaying malformed parameter errors
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::Error::*;

        match *self {
            UnknownCoreMethod(ref method) => write!(f, "Error: Unknown core method '{}'", method),
            MalformedCoreParams(ref method, ref params) =>
                write!(f, "Error: Malformed core parameters with method '{}', parameters: {:?}", method, params),
            UnknownEditMethod(ref method) => write!(f, "Error: Unknown edit method '{}'", method),
            MalformedEditParams(ref method, ref params) =>
                write!(f, "Error: Malformed edit parameters with method '{}', parameters: {:?}", method, params),
        }
    }
}

impl error::Error for Error {
    fn description(&self) -> &str {
        use self::Error::*;

        match *self {
            UnknownCoreMethod(_) => "Unknown core method",
            MalformedCoreParams(_, _) => "Malformed core parameters",
            UnknownEditMethod(_) => "Unknown edit method",
            MalformedEditParams(_, _) => "Malformed edit parameters"
        }
    }
}
