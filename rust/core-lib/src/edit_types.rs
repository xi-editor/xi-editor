// Copyright 2018 The xi-editor Authors.
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

//! A bunch of boilerplate for converting the `EditNotification`s we receive
//! from the client into the events we use internally.
//!
//! This simplifies code elsewhere, and makes it easier to route events to
//! the editor or view as appropriate.

use crate::movement::Movement;
use crate::rpc::{
    EditNotification, FindQuery, GestureType, LineRange, MouseAction, Position,
    SelectionGranularity, SelectionModifier,
};
use crate::view::Size;

/// Events that only modify view state
#[derive(Debug, PartialEq, Clone)]
pub(crate) enum ViewEvent {
    Move(Movement),
    ModifySelection(Movement),
    SelectAll,
    Scroll(LineRange),
    AddSelectionAbove,
    AddSelectionBelow,
    Click(MouseAction),
    Drag(MouseAction),
    Gesture { line: u64, col: u64, ty: GestureType },
    GotoLine { line: u64 },
    Find { chars: String, case_sensitive: bool, regex: bool, whole_words: bool },
    MultiFind { queries: Vec<FindQuery> },
    FindNext { wrap_around: bool, allow_same: bool, modify_selection: SelectionModifier },
    FindPrevious { wrap_around: bool, allow_same: bool, modify_selection: SelectionModifier },
    FindAll,
    HighlightFind { visible: bool },
    SelectionForFind { case_sensitive: bool },
    Replace { chars: String, preserve_case: bool },
    SelectionForReplace,
    SelectionIntoLines,
    CollapseSelections,
}

/// Events that modify the buffer
#[derive(Debug, PartialEq, Clone)]
pub(crate) enum BufferEvent {
    Delete { movement: Movement, kill: bool },
    Backspace,
    Transpose,
    Undo,
    Redo,
    Uppercase,
    Lowercase,
    Capitalize,
    Indent,
    Outdent,
    Insert(String),
    Paste(String),
    InsertNewline,
    InsertTab,
    Yank,
    ReplaceNext,
    ReplaceAll,
    DuplicateLine,
    IncreaseNumber,
    DecreaseNumber,
}

/// An event that needs special handling
#[derive(Debug, PartialEq, Clone)]
pub(crate) enum SpecialEvent {
    DebugRewrap,
    DebugWrapWidth,
    DebugPrintSpans,
    Resize(Size),
    RequestLines(LineRange),
    RequestHover { request_id: usize, position: Option<Position> },
    DebugToggleComment,
    Reindent,
    ToggleRecording(Option<String>),
    PlayRecording(String),
    ClearRecording(String),
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum EventDomain {
    View(ViewEvent),
    Buffer(BufferEvent),
    Special(SpecialEvent),
}

impl From<BufferEvent> for EventDomain {
    fn from(src: BufferEvent) -> EventDomain {
        EventDomain::Buffer(src)
    }
}

impl From<ViewEvent> for EventDomain {
    fn from(src: ViewEvent) -> EventDomain {
        EventDomain::View(src)
    }
}

impl From<SpecialEvent> for EventDomain {
    fn from(src: SpecialEvent) -> EventDomain {
        EventDomain::Special(src)
    }
}

#[rustfmt::skip]
impl From<EditNotification> for EventDomain {
    fn from(src: EditNotification) -> EventDomain {
        use self::EditNotification::*;
        match src {
            Insert { chars } =>
                BufferEvent::Insert(chars).into(),
            Paste { chars } =>
                BufferEvent::Paste(chars).into(),
            DeleteForward =>
                BufferEvent::Delete {
                    movement: Movement::Right,
                    kill: false
                }.into(),
            DeleteBackward =>
                BufferEvent::Backspace.into(),
            DeleteWordForward =>
                BufferEvent::Delete {
                    movement: Movement::RightWord,
                    kill: false
                }.into(),
            DeleteWordBackward =>
                BufferEvent::Delete {
                    movement: Movement::LeftWord,
                    kill: false
                }.into(),
            DeleteToEndOfParagraph =>
                BufferEvent::Delete {
                    movement: Movement::EndOfParagraphKill,
                    kill: true
                }.into(),
            DeleteToBeginningOfLine =>
                BufferEvent::Delete {
                    movement: Movement::LeftOfLine,
                    kill: false
                }.into(),
            InsertNewline =>
                BufferEvent::InsertNewline.into(),
            InsertTab =>
                BufferEvent::InsertTab.into(),
            MoveUp =>
                ViewEvent::Move(Movement::Up).into(),
            MoveUpAndModifySelection =>
                ViewEvent::ModifySelection(Movement::Up).into(),
            MoveDown =>
                ViewEvent::Move(Movement::Down).into(),
            MoveDownAndModifySelection =>
                ViewEvent::ModifySelection(Movement::Down).into(),
            MoveLeft | MoveBackward =>
                ViewEvent::Move(Movement::Left).into(),
            MoveLeftAndModifySelection =>
                ViewEvent::ModifySelection(Movement::Left).into(),
            MoveRight | MoveForward  =>
                ViewEvent::Move(Movement::Right).into(),
            MoveRightAndModifySelection =>
                ViewEvent::ModifySelection(Movement::Right).into(),
            MoveWordLeft =>
                ViewEvent::Move(Movement::LeftWord).into(),
            MoveWordLeftAndModifySelection =>
                ViewEvent::ModifySelection(Movement::LeftWord).into(),
            MoveWordRight =>
                ViewEvent::Move(Movement::RightWord).into(),
            MoveWordRightAndModifySelection =>
                ViewEvent::ModifySelection(Movement::RightWord).into(),
            MoveToBeginningOfParagraph =>
                ViewEvent::Move(Movement::StartOfParagraph).into(),
            MoveToBeginningOfParagraphAndModifySelection =>
                ViewEvent::ModifySelection(Movement::StartOfParagraph).into(),
            MoveToEndOfParagraph =>
                ViewEvent::Move(Movement::EndOfParagraph).into(),
            MoveToEndOfParagraphAndModifySelection =>
                ViewEvent::ModifySelection(Movement::EndOfParagraph).into(),
            MoveToLeftEndOfLine =>
                ViewEvent::Move(Movement::LeftOfLine).into(),
            MoveToLeftEndOfLineAndModifySelection =>
                ViewEvent::ModifySelection(Movement::LeftOfLine).into(),
            MoveToRightEndOfLine =>
                ViewEvent::Move(Movement::RightOfLine).into(),
            MoveToRightEndOfLineAndModifySelection =>
                ViewEvent::ModifySelection(Movement::RightOfLine).into(),
            MoveToBeginningOfDocument =>
                ViewEvent::Move(Movement::StartOfDocument).into(),
            MoveToBeginningOfDocumentAndModifySelection =>
                ViewEvent::ModifySelection(Movement::StartOfDocument).into(),
            MoveToEndOfDocument =>
                ViewEvent::Move(Movement::EndOfDocument).into(),
            MoveToEndOfDocumentAndModifySelection =>
                ViewEvent::ModifySelection(Movement::EndOfDocument).into(),
            ScrollPageUp =>
                ViewEvent::Move(Movement::UpPage).into(),
            PageUpAndModifySelection =>
                ViewEvent::ModifySelection(Movement::UpPage).into(),
            ScrollPageDown =>
                ViewEvent::Move(Movement::DownPage).into(),
            PageDownAndModifySelection =>
                ViewEvent::ModifySelection(Movement::DownPage).into(),
            SelectAll => ViewEvent::SelectAll.into(),
            AddSelectionAbove => ViewEvent::AddSelectionAbove.into(),
            AddSelectionBelow => ViewEvent::AddSelectionBelow.into(),
            Scroll(range) => ViewEvent::Scroll(range).into(),
            Resize(size) => SpecialEvent::Resize(size).into(),
            GotoLine { line } => ViewEvent::GotoLine { line }.into(),
            RequestLines(range) => SpecialEvent::RequestLines(range).into(),
            Yank => BufferEvent::Yank.into(),
            Transpose => BufferEvent::Transpose.into(),
            Click(action) => ViewEvent::Click(action).into(),
            Drag(action) => ViewEvent::Drag(action).into(),
            Gesture { line, col,  ty } => {
                // Translate deprecated gesture types into the new format
                let new_ty = match ty {
                    GestureType::PointSelect => {
                        warn!("The point_select gesture is deprecated; use select instead");
                        GestureType::Select {granularity: SelectionGranularity::Point, multi: false}
                    }
                    GestureType::ToggleSel => {
                        warn!("The toggle_sel gesture is deprecated; use select instead");
                        GestureType::Select { granularity: SelectionGranularity::Point, multi: true}
                    }
                    GestureType::WordSelect => {
                        warn!("The word_select gesture is deprecated; use select instead");
                        GestureType::Select { granularity: SelectionGranularity::Word, multi: false}
                    }
                    GestureType::MultiWordSelect => {
                        warn!("The multi_word_select gesture is deprecated; use select instead");
                        GestureType::Select { granularity: SelectionGranularity::Word, multi: true}
                    }
                    GestureType::LineSelect => {
                        warn!("The line_select gesture is deprecated; use select instead");
                        GestureType::Select { granularity: SelectionGranularity::Line, multi: false}
                    }
                    GestureType::MultiLineSelect => {
                        warn!("The multi_line_select gesture is deprecated; use select instead");
                        GestureType::Select { granularity: SelectionGranularity::Line, multi: true}
                    }
                    GestureType::RangeSelect => {
                        warn!("The range_select gesture is deprecated; use select_extend instead");
                        GestureType::SelectExtend { granularity: SelectionGranularity::Point }
                    }
                    _ => ty
                };
                ViewEvent::Gesture { line, col, ty: new_ty }.into()
            },
            Undo => BufferEvent::Undo.into(),
            Redo => BufferEvent::Redo.into(),
            Find { chars, case_sensitive, regex, whole_words } =>
                ViewEvent::Find { chars, case_sensitive, regex, whole_words }.into(),
            MultiFind { queries } =>
                ViewEvent::MultiFind { queries }.into(),
            FindNext { wrap_around, allow_same, modify_selection } =>
                ViewEvent::FindNext { wrap_around, allow_same, modify_selection }.into(),
            FindPrevious { wrap_around, allow_same, modify_selection } =>
                ViewEvent::FindPrevious { wrap_around, allow_same, modify_selection }.into(),
            FindAll => ViewEvent::FindAll.into(),
            DebugRewrap => SpecialEvent::DebugRewrap.into(),
            DebugWrapWidth => SpecialEvent::DebugWrapWidth.into(),
            DebugPrintSpans => SpecialEvent::DebugPrintSpans.into(),
            Uppercase => BufferEvent::Uppercase.into(),
            Lowercase => BufferEvent::Lowercase.into(),
            Capitalize => BufferEvent::Capitalize.into(),
            Indent => BufferEvent::Indent.into(),
            Outdent => BufferEvent::Outdent.into(),
            Reindent => SpecialEvent::Reindent.into(),
            DebugToggleComment => SpecialEvent::DebugToggleComment.into(),
            HighlightFind { visible } => ViewEvent::HighlightFind { visible }.into(),
            SelectionForFind { case_sensitive } =>
                ViewEvent::SelectionForFind { case_sensitive }.into(),
            Replace { chars, preserve_case } =>
                ViewEvent::Replace { chars, preserve_case }.into(),
            ReplaceNext => BufferEvent::ReplaceNext.into(),
            ReplaceAll => BufferEvent::ReplaceAll.into(),
            SelectionForReplace => ViewEvent::SelectionForReplace.into(),
            RequestHover { request_id, position } =>
                SpecialEvent::RequestHover { request_id, position }.into(),
            SelectionIntoLines => ViewEvent::SelectionIntoLines.into(),
            DuplicateLine => BufferEvent::DuplicateLine.into(),
            IncreaseNumber => BufferEvent::IncreaseNumber.into(),
            DecreaseNumber => BufferEvent::DecreaseNumber.into(),
            ToggleRecording { recording_name } => SpecialEvent::ToggleRecording(recording_name).into(),
            PlayRecording { recording_name } => SpecialEvent::PlayRecording(recording_name).into(),
            ClearRecording { recording_name } => SpecialEvent::ClearRecording(recording_name).into(),
            CollapseSelections => ViewEvent::CollapseSelections.into(),
        }
    }
}
