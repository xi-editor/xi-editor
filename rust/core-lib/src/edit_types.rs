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

use movement::Movement;
use ::rpc::{GestureType, LineRange, EditNotification, MouseAction};


/// Events that only modify view state
pub (crate) enum ViewEvent {
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
    FindNext { wrap_around: Option<bool>, allow_same: Option<bool> },
    FindPrevious { wrap_around: Option<bool> },
    Cancel,
}

/// Events that modify the buffer
pub (crate) enum BufferEvent {
    Delete(Movement),
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
    RequestLines(LineRange),
    Yank,
    DebugRewrap,
    DebugPrintSpans,
}

pub (crate) enum EventDomain {
    View(ViewEvent),
    Buffer(BufferEvent),
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

impl From<EditNotification> for EventDomain {
    fn from(src: EditNotification) -> EventDomain {
        use self::EditNotification::*;
        match src {
            Insert { chars } =>
                BufferEvent::Insert(chars).into(),
            DeleteForward =>
                BufferEvent::Delete(Movement::Left).into(),
            DeleteBackward =>
                BufferEvent::Backspace.into(),
            DeleteWordForward =>
                BufferEvent::Delete(Movement::RightWord).into(),
            DeleteWordBackward =>
                BufferEvent::Delete(Movement::LeftWord).into(),
            DeleteToEndOfParagraph =>
                BufferEvent::Delete(Movement::EndOfParagraphKill).into(),
            DeleteToBeginningOfLine =>
                BufferEvent::Delete(Movement::LeftOfLine).into(),
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
                ViewEvent::Move(Movement::Left).into(),
            MoveWordLeftAndModifySelection =>
                ViewEvent::ModifySelection(Movement::Left).into(),
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
            GotoLine { line } => ViewEvent::GotoLine { line }.into(),
            RequestLines(range) => BufferEvent::RequestLines(range).into(),
            Yank => BufferEvent::Yank.into(),
            Transpose => BufferEvent::Transpose.into(),
            Click(action) => ViewEvent::Click(action).into(),
            Drag(action) => ViewEvent::Drag(action).into(),
            Gesture { line, col,  ty } =>
                ViewEvent::Gesture { line, col, ty }.into(),
            Undo => BufferEvent::Undo.into(),
            Redo => BufferEvent::Redo.into(),
            FindNext { wrap_around, allow_same } =>
                ViewEvent::FindNext { wrap_around, allow_same }.into(),
            FindPrevious { wrap_around } =>
                ViewEvent::FindPrevious { wrap_around }.into(),
            DebugRewrap => BufferEvent::DebugRewrap.into(),
            DebugPrintSpans => BufferEvent::DebugPrintSpans.into(),
            CancelOperation => ViewEvent::Cancel.into(),
            Uppercase => BufferEvent::Uppercase.into(),
            Lowercase => BufferEvent::Lowercase.into(),
            Indent => BufferEvent::Indent.into(),
            Outdent => BufferEvent::Outdent.into(),
        }
    }
}

