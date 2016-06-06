use serde_json::Value;
use std::collections::BTreeMap;
use std::error;
use std::fmt;

/// An enum representing an edit command, parsed from JSON.
#[derive(Debug, PartialEq, Eq)]
pub enum EditCommand {
    RenderLines(usize, usize), // first line, last line
    Key(String, u64), // chars, flags
    Insert(String), // chars
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
    Open(String), // file path
    Save(String), // file path
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
    UnknownMethod,
    MalformedParams,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Error::UnknownMethod => write!(f, "Error: Unknown method"),
            Error::MalformedParams => write!(f, "Error: Malformed parameters"),
        }
    }
}

impl error::Error for Error {
    fn description(&self) -> &str {
        match *self {
            Error::UnknownMethod => "Unknown method",
            Error::MalformedParams => "Malformed parameters"
        }
    }
}

impl EditCommand {
    /// Try to read an edit command with the given method and parameters.
    pub fn from_json(method: &str, params: &Value) -> Result<Self, Error> {
        use self::EditCommand::*;
        use self::Error::*;

        fn get_u64(dict: &BTreeMap<String, Value>, key: &str) -> Option<u64> {
            dict.get(key).and_then(|v| v.as_u64())
        }

        fn get_string<'a>(dict: &'a BTreeMap<String, Value>, key: &str) -> Option<&'a str> {
            dict.get(key).and_then(|v| v.as_string())
        }

        fn arr_get_u64(arr: &[Value], idx: usize) -> Option<u64> {
            arr.get(idx).and_then(Value::as_u64)
        }

        fn arr_get_i64(arr: &[Value], idx: usize) -> Option<i64> {
            arr.get(idx).and_then(Value::as_i64)
        }

        match method {
            "render_lines" => {
                params.as_object().and_then(|dict| {
                    get_u64(dict, "first_line").and_then(|first_line| {
                        get_u64(dict, "last_line").map(|last_line| {
                            RenderLines(first_line as usize, last_line as usize)
                        })
                    })
                }).ok_or(MalformedParams)
            },
            "key" => params.as_object().and_then(|dict| {
                get_string(dict, "chars").and_then(|chars| {
                    get_u64(dict, "flags").map(|flags| {
                        Key(chars.to_string(), flags)
                    })
                })
            }).ok_or(MalformedParams),
            "insert" => params.as_object().and_then(|dict| {
                get_string(dict, "chars").map(|chars| Insert(chars.to_string()))
            }).ok_or(MalformedParams),
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
                get_string(dict, "filename").map(|path| Open(path.to_string()))
            }).ok_or(MalformedParams),
            "save" => params.as_object().and_then(|dict| {
                get_string(dict, "filename").map(|path| Save(path.to_string()))
            }).ok_or(MalformedParams),
            "scroll" => params.as_array().and_then(|arr| {
                if let (Some(first), Some(last)) =
                    (arr_get_i64(arr, 0), arr_get_i64(arr, 1)) {

                    Some(Scroll(first, last))
                } else { None }
            }).ok_or(MalformedParams),
            "yank" => Ok(Yank),
            "transpose" => Ok(Transpose),
            "click" => params.as_array().and_then(|arr| {
                if let (Some(line), Some(col), Some(flags), Some(click_count)) =
                    (arr_get_u64(arr, 0), arr_get_u64(arr, 1), arr_get_u64(arr, 2), arr_get_u64(arr, 3)) {

                        Some(Click(line, col, flags, click_count))
                    } else { None }
            }).ok_or(MalformedParams),
            "drag" => params.as_array().and_then(|arr| {
                if let (Some(line), Some(col), Some(flags)) =
                    (arr_get_u64(arr, 0), arr_get_u64(arr, 1), arr_get_u64(arr, 2)) {

                        Some(Drag(line, col, flags))
                    } else { None }
            }).ok_or(MalformedParams),
            "undo" => Ok(Undo),
            "redo" => Ok(Redo),
            "cut" => Ok(Cut),
            "copy" => Ok(Copy),
            "debug_rewrap" => Ok(DebugRewrap),
            "debug_test_fg_spans" => Ok(DebugTestFgSpans),
            _ => Err(UnknownMethod),
        }
    }
}
