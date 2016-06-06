use serde_json::Value;
use std::collections::BTreeMap;
use std::error;
use std::fmt;

/// An enum representing a tab command, parsed from JSON.
#[derive(Debug, PartialEq, Eq)]
pub enum TabCommand<'a> {
    Edit(&'a str, EditCommand<'a>), // tab name, edit command
    NewTab,
    DeleteTab(&'a str), // tab name
}

/// An enum representing an edit command, parsed from JSON.
#[derive(Debug, PartialEq, Eq)]
pub enum EditCommand<'a> {
    RenderLines(usize, usize), // first line, last line
    Key(&'a str, u64), // chars, flags
    Insert(&'a str), // chars
    DeleteBackward,
    DeleteToEndOfParagraph,
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
    ScrollPageUp,
    PageUpAndModifySelection,
    ScrollPageDown,
    PageDownAndModifySelection,
    Open(&'a str), // file path
    Save(&'a str), // file path
    Scroll(i64, i64), // first, last
    Yank,
    Transpose,
    Click(u64, u64, u64, u64), // line, column, flags, click count
    Drag(u64, u64, u64), // line, col, flags
    Undo,
    Redo,
    Cut,
    Copy,
    DebugRewrap,
    DebugTestFgSpans,
}

/// An error that occurred while parsing an edit command.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    UnknownTabMethod,
    MalformedTabParams,
    UnknownEditMethod,
    MalformedEditParams,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::Error::*;

        match *self {
            UnknownTabMethod => write!(f, "Error: Unknown tab method"),
            MalformedTabParams => write!(f, "Error: Malformed tab parameters"),
            UnknownEditMethod => write!(f, "Error: Unknown edit method"),
            MalformedEditParams => write!(f, "Error: Malformed edit parameters"),
        }
    }
}

impl error::Error for Error {
    fn description(&self) -> &str {
        use self::Error::*;

        match *self {
            UnknownTabMethod => "Unknown tab method",
            MalformedTabParams => "Malformed tab parameters",
            UnknownEditMethod => "Unknown edit method",
            MalformedEditParams => "Malformed edit parameters"
        }
    }
}

impl<'a> TabCommand<'a> {
    pub fn from_json(method: &str, params: &'a Value) -> Result<Self, Error> {
        use self::TabCommand::*;
        use self::Error::*;

        match method {
            "new_tab" => Ok(NewTab),
            "delete_tab" => params.as_object().and_then(|dict| {
                dict_get_string(dict, "tab").map(|tab| DeleteTab(tab))
            }).ok_or(MalformedTabParams),
            "edit" => params.as_object().ok_or(MalformedTabParams).and_then(|dict| {
                if let (Some(tab), Some(method), Some(edit_params)) =
                    (dict_get_string(dict, "tab"), dict_get_string(dict, "method"), dict.get("params")) {
                        EditCommand::from_json(method, edit_params).map(|cmd| Edit(tab, cmd))
                    } else { Err(MalformedTabParams) }
            }),
            _ => Err(UnknownTabMethod)
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
                    dict_get_u64(dict, "first_line").and_then(|first_line| {
                        dict_get_u64(dict, "last_line").map(|last_line| {
                            RenderLines(first_line as usize, last_line as usize)
                        })
                    })
                }).ok_or(MalformedEditParams)
            },
            "key" => params.as_object().and_then(|dict| {
                dict_get_string(dict, "chars").and_then(|chars| {
                    dict_get_u64(dict, "flags").map(|flags| {
                        Key(chars, flags)
                    })
                })
            }).ok_or(MalformedEditParams),
            "insert" => params.as_object().and_then(|dict| {
                dict_get_string(dict, "chars").map(|chars| Insert(chars))
            }).ok_or(MalformedEditParams),
            "delete_backward" => Ok(DeleteBackward),
            "delete_to_end_of_paragraph" => Ok(DeleteToEndOfParagraph),
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
            "scroll_page_up" | "page_up" => Ok(ScrollPageUp),
            "page_up_and_modify_selection" => Ok(PageUpAndModifySelection),
            "scroll_page_down" |
            "page_down" => Ok(ScrollPageDown),
            "page_down_and_modify_selection" => Ok(PageDownAndModifySelection),
            "open" => params.as_object().and_then(|dict| {
                dict_get_string(dict, "filename").map(|path| Open(path))
            }).ok_or(MalformedEditParams),
            "save" => params.as_object().and_then(|dict| {
                dict_get_string(dict, "filename").map(|path| Save(path))
            }).ok_or(MalformedEditParams),
            "scroll" => params.as_array().and_then(|arr| {
                if let (Some(first), Some(last)) =
                    (arr_get_i64(arr, 0), arr_get_i64(arr, 1)) {

                    Some(Scroll(first, last))
                } else { None }
            }).ok_or(MalformedEditParams),
            "yank" => Ok(Yank),
            "transpose" => Ok(Transpose),
            "click" => params.as_array().and_then(|arr| {
                if let (Some(line), Some(col), Some(flags), Some(click_count)) =
                    (arr_get_u64(arr, 0), arr_get_u64(arr, 1), arr_get_u64(arr, 2), arr_get_u64(arr, 3)) {

                        Some(Click(line, col, flags, click_count))
                    } else { None }
            }).ok_or(MalformedEditParams),
            "drag" => params.as_array().and_then(|arr| {
                if let (Some(line), Some(col), Some(flags)) =
                    (arr_get_u64(arr, 0), arr_get_u64(arr, 1), arr_get_u64(arr, 2)) {

                        Some(Drag(line, col, flags))
                    } else { None }
            }).ok_or(MalformedEditParams),
            "undo" => Ok(Undo),
            "redo" => Ok(Redo),
            "cut" => Ok(Cut),
            "copy" => Ok(Copy),
            "debug_rewrap" => Ok(DebugRewrap),
            "debug_test_fg_spans" => Ok(DebugTestFgSpans),
            _ => Err(UnknownEditMethod),
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
