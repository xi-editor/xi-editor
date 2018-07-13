// Copyright 2018 Google Inc. All rights reserved.
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

use internal::plugins::rpc::Position;
use movement::Movement;
use rpc::{GestureType, LineRange, EditNotification, MouseAction, SelectionModifier};
use view::Size;


/// Events that only modify view state
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
    FindNext { wrap_around: bool, allow_same: bool, modify_selection: SelectionModifier },
    FindPrevious { wrap_around: bool, allow_same: bool, modify_selection: SelectionModifier },
    FindAll,
    Cancel,
    HighlightFind { visible: bool },
    SelectionForFind { case_sensitive: bool },
    Replace { chars: String, preserve_case: bool },
    SelectionForReplace,
}

/// Events that modify the buffer
pub(crate) enum BufferEvent {
    Delete { movement: Movement, kill: bool },
    Backspace,
    Transpose,
    Undo,
    Redo,
    Uppercase,
    Lowercase,
    Indent,
    Outdent,
    Insert(String),
    InsertNewline,
    InsertTab,
    Yank,
    ReplaceNext,
    ReplaceAll,
}

/// An event that needs special handling
pub(crate) enum SpecialEvent {
    DebugRewrap,
    DebugWrapWidth,
    DebugPrintSpans,
    Resize(Size),
    RequestLines(LineRange),
    RequestHover { request_id: usize, position: Option<Position> },
    RequestDefinition { request_id: usize, position: Option<Position> }
}

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

impl From<EditNotification> for EventDomain {
    fn from(src: EditNotification) -> EventDomain {
        use self::EditNotification::*;
        match src {
            Insert { chars } =>
                BufferEvent::Insert(chars).into(),
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
            MoveToEndOfParagraph =>
                ViewEvent::Move(Movement::EndOfParagraph).into(),
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
            Gesture { line, col,  ty } =>
                ViewEvent::Gesture { line, col, ty }.into(),
            Undo => BufferEvent::Undo.into(),
            Redo => BufferEvent::Redo.into(),
            Find { chars, case_sensitive, regex, whole_words } =>
                ViewEvent::Find { chars, case_sensitive, regex, whole_words }.into(),
            FindNext { wrap_around, allow_same, modify_selection } =>
                ViewEvent::FindNext { wrap_around, allow_same, modify_selection }.into(),
            FindPrevious { wrap_around, allow_same, modify_selection } =>
                ViewEvent::FindPrevious { wrap_around, allow_same, modify_selection }.into(),
            FindAll => ViewEvent::FindAll.into(),
            DebugRewrap => SpecialEvent::DebugRewrap.into(),
            DebugWrapWidth => SpecialEvent::DebugWrapWidth.into(),
            DebugPrintSpans => SpecialEvent::DebugPrintSpans.into(),
            CancelOperation => ViewEvent::Cancel.into(),
            Uppercase => BufferEvent::Uppercase.into(),
            Lowercase => BufferEvent::Lowercase.into(),
            Indent => BufferEvent::Indent.into(),
            Outdent => BufferEvent::Outdent.into(),
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
            RequestDefinition { request_id, position } =>
                SpecialEvent::RequestDefinition { request_id, position }.into()
        }
    }
}

