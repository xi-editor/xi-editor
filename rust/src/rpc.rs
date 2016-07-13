use std::io;
use std::collections::BTreeMap;
use std::io::Write;
use std::error;
use std::fmt;
use serde_json;
use serde_json::builder::ObjectBuilder;
use serde_json::Value;

// =============================================================================
//  Request handling
// =============================================================================

pub fn send(v: &Value) -> Result<(), io::Error> {
    let mut s = serde_json::to_string(v).unwrap();
    s.push('\n');
    //print_err!("from core: {}", s);
    io::stdout().write_all(s.as_bytes())
}

pub fn respond(result: &Value, id: Option<&Value>)
{
    if let Some(id) = id {
        if let Err(e) = send(&ObjectBuilder::new()
                             .insert("id", id)
                             .insert("result", result)
                             .unwrap()) {
            print_err!("error {} sending response to RPC {:?}", e, id);
        }
    } else {
        print_err!("tried to respond with no id");
    }
}

impl<'a> Request<'a> {
    pub fn from_json(val: &'a Value) -> Result<Self, Error> {
        use self::Error::*;

        val.as_object().ok_or(InvalidRequest).and_then(|req| {
            if let (Some(method), Some(params)) =
                (dict_get_string(req, "method"), req.get("params")) {

                    let id = req.get("id");
                    TabCommand::from_json(method, params).map(|cmd| Request::TabCommand { id: id, tab_command: cmd})
                }
            else { Err(InvalidRequest) }
        })
    }
}

// =============================================================================
//  Command types
// =============================================================================

#[derive(Debug, PartialEq)]
pub enum Request<'a> {
    TabCommand { id: Option<&'a Value>, tab_command: TabCommand<'a> }
}

/// An enum representing a tab command, parsed from JSON.
#[derive(Debug, PartialEq, Eq)]
pub enum TabCommand<'a> {
    Edit { tab_name: &'a str, edit_command: EditCommand<'a> },
    NewTab,
    DeleteTab { tab_name: &'a str },
}

/// An enum representing an edit command, parsed from JSON.
#[derive(Debug, PartialEq, Eq)]
pub enum EditCommand<'a> {
    RenderLines { first_line: usize, last_line: usize },
    Key { chars: &'a str, flags: u64 },
    Insert { chars: &'a str },
    DeleteForward,
    DeleteBackward,
    DeleteToEndOfParagraph,
    DeleteToBeginningOfLine,
    InsertNewline,
    MoveUp,
    MoveUpAndModifySelection,
    MoveDown,
    MoveDownAndModifySelection,
    MoveLeft,
    MoveLeftAndModifySelection,
    MoveRight,
    MoveRightAndModifySelection,
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
    Open { file_path: &'a str },
    Save { file_path: &'a str },
    Scroll { first: i64, last: i64 },
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
}

impl<'a> TabCommand<'a> {
    pub fn from_json(method: &str, params: &'a Value) -> Result<Self, Error> {
        use self::TabCommand::*;
        use self::Error::*;

        match method {
            "new_tab" => Ok(NewTab),

            "delete_tab" => params.as_object().and_then(|dict| {
                dict_get_string(dict, "tab").map(|tab_name| DeleteTab { tab_name: tab_name })
            }).ok_or(MalformedTabParams(method.to_string(), params.clone())),

            "edit" =>
                params
                .as_object()
                .ok_or(MalformedTabParams(method.to_string(), params.clone()))
                .and_then(|dict| {
                    if let (Some(tab), Some(method), Some(edit_params)) =
                        (dict_get_string(dict, "tab"), dict_get_string(dict, "method"), dict.get("params")) {
                            EditCommand::from_json(method, edit_params)
                                .map(|cmd| Edit { tab_name: tab, edit_command: cmd })
                        } else { Err(MalformedTabParams(method.to_string(), params.clone())) }
            }),

            _ => Err(UnknownTabMethod(method.to_string()))
        }
    }
}

impl<'a> EditCommand<'a> {
    /// Try to read an edit command with the given method and parameters.
    pub fn from_json(method: &str, params: &'a Value) -> Result<Self, Error> {
        use self::EditCommand::*;
        use self::Error::*;

        match method {
            "render_lines" => {
                params.as_object().and_then(|dict| {
                    if let (Some(first_line), Some(last_line)) =
                        (dict_get_u64(dict, "first_line"), dict_get_u64(dict, "last_line")) {
                            Some(RenderLines {
                                first_line: first_line as usize,
                                last_line: last_line as usize
                            })
                        } else { None }
                }).ok_or(MalformedEditParams(method.to_string(), params.clone()))
            },

            "key" => params.as_object().and_then(|dict| {
                dict_get_string(dict, "chars").and_then(|chars| {
                    dict_get_u64(dict, "flags").map(|flags| {
                        Key { chars: chars, flags: flags }
                    })
                })
            }).ok_or(MalformedEditParams(method.to_string(), params.clone())),

            "insert" => params.as_object().and_then(|dict| {
                dict_get_string(dict, "chars").map(|chars| Insert { chars: chars })
            }).ok_or(MalformedEditParams(method.to_string(), params.clone())),

            "delete_forward" => Ok(DeleteForward),
            "delete_backward" => Ok(DeleteBackward),
            "delete_to_end_of_paragraph" => Ok(DeleteToEndOfParagraph),
            "delete_to_beginning_of_line" => Ok(DeleteToBeginningOfLine),
            "insert_newline" => Ok(InsertNewline),
            "move_up" => Ok(MoveUp),
            "move_up_and_modify_selection" => Ok(MoveUpAndModifySelection),
            "move_down" => Ok(MoveDown),
            "move_down_and_modify_selection" => Ok(MoveDownAndModifySelection),
            "move_left" | "move_backward" => Ok(MoveLeft),
            "move_left_and_modify_selection" => Ok(MoveLeftAndModifySelection),
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

            "open" => params.as_object().and_then(|dict| {
                dict_get_string(dict, "filename").map(|path| Open { file_path: path })
            }).ok_or(MalformedEditParams(method.to_string(), params.clone())),

            "save" => params.as_object().and_then(|dict| {
                dict_get_string(dict, "filename").map(|path| Save { file_path: path })
            }).ok_or(MalformedEditParams(method.to_string(), params.clone())),

            "scroll" => params.as_array().and_then(|arr| {
                if let (Some(first), Some(last)) =
                    (arr_get_i64(arr, 0), arr_get_i64(arr, 1)) {

                    Some(Scroll { first: first, last: last })
                } else { None }
            }).ok_or(MalformedEditParams(method.to_string(), params.clone())),

            "yank" => Ok(Yank),
            "transpose" => Ok(Transpose),

            "click" => params.as_array().and_then(|arr| {
                if let (Some(line), Some(column), Some(flags), Some(click_count)) =
                    (arr_get_u64(arr, 0), arr_get_u64(arr, 1), arr_get_u64(arr, 2), arr_get_u64(arr, 3)) {

                        Some(Click { line: line, column: column, flags: flags, click_count: click_count })
                    } else { None }
            }).ok_or(MalformedEditParams(method.to_string(), params.clone())),

            "drag" => params.as_array().and_then(|arr| {
                if let (Some(line), Some(column), Some(flags)) =
                    (arr_get_u64(arr, 0), arr_get_u64(arr, 1), arr_get_u64(arr, 2)) {

                        Some(Drag { line: line, column: column, flags: flags })
                    } else { None }
            }).ok_or(MalformedEditParams(method.to_string(), params.clone())),

            "undo" => Ok(Undo),
            "redo" => Ok(Redo),
            "cut" => Ok(Cut),
            "copy" => Ok(Copy),
            "debug_rewrap" => Ok(DebugRewrap),
            "debug_test_fg_spans" => Ok(DebugTestFgSpans),
            "debug_run_plugin" => Ok(DebugRunPlugin),

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
    InvalidRequest,
    UnknownTabMethod(String), // method name
    MalformedTabParams(String, Value), // method name, malformed params
    UnknownEditMethod(String), // method name
    MalformedEditParams(String, Value), // method name, malformed params
}

impl fmt::Display for Error {
    // TODO: Provide information about the parameter format expected when
    // displaying malformed parameter errors
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::Error::*;

        match *self {
            InvalidRequest => write!(f, "Error: invalid request"),
            UnknownTabMethod(ref method) => write!(f, "Error: Unknown tab method '{}'", method),
            MalformedTabParams(ref method, ref params) =>
                write!(f, "Error: Malformed tab parameters with method '{}', parameters: {:?}", method, params),
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
            InvalidRequest => "Invalid request",
            UnknownTabMethod(_) => "Unknown tab method",
            MalformedTabParams(_, _) => "Malformed tab parameters",
            UnknownEditMethod(_) => "Unknown edit method",
            MalformedEditParams(_, _) => "Malformed edit parameters"
        }
    }
}

// =============================================================================
//  Helper functions for value access
// =============================================================================

fn dict_get_u64(dict: &BTreeMap<String, Value>, key: &str) -> Option<u64> {
    dict.get(key).and_then(|v| v.as_u64())
}

fn dict_get_string<'a>(dict: &'a BTreeMap<String, Value>, key: &str) -> Option<&'a str> {
    dict.get(key).and_then(|v| v.as_string())
}

fn arr_get_u64(arr: &[Value], idx: usize) -> Option<u64> {
    arr.get(idx).and_then(Value::as_u64)
}

fn arr_get_i64(arr: &[Value], idx: usize) -> Option<i64> {
    arr.get(idx).and_then(Value::as_i64)
}
