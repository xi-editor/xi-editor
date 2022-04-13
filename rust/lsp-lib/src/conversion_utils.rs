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

//! Utility functions meant for converting types from LSP to Core format
//! and vice-versa

use crate::lsp_types::*;
use crate::types::LanguageResponseError;
use xi_plugin_lib::{Cache, Error as PluginLibError, Hover as CoreHover, Range as CoreRange, View};

pub(crate) fn marked_string_to_string(marked_string: &MarkedString) -> String {
    match *marked_string {
        MarkedString::String(ref text) => text.to_owned(),
        MarkedString::LanguageString(ref d) => format!("```{}\n{}\n```", d.language, d.value),
    }
}

pub(crate) fn markdown_from_hover_contents(
    hover_contents: HoverContents,
) -> Result<String, LanguageResponseError> {
    let res = match hover_contents {
        HoverContents::Scalar(content) => marked_string_to_string(&content),
        HoverContents::Array(content) => {
            let res: Vec<String> = content.iter().map(marked_string_to_string).collect();
            res.join("\n")
        }
        HoverContents::Markup(content) => content.value,
    };
    if res.is_empty() {
        Err(LanguageResponseError::FallbackResponse)
    } else {
        Ok(res)
    }
}

/// Counts the number of utf-16 code units in the given string.
pub(crate) fn count_utf16(s: &str) -> usize {
    let mut utf16_count = 0;
    for &b in s.as_bytes() {
        if (b as i8) >= -0x40 {
            utf16_count += 1;
        }
        if b >= 0xf0 {
            utf16_count += 1;
        }
    }
    utf16_count
}

/// Get LSP Style Utf-16 based position given the xi-core style utf-8 offset
pub(crate) fn get_position_of_offset<C: Cache>(
    view: &mut View<C>,
    offset: usize,
) -> Result<Position, PluginLibError> {
    let line_num = view.line_of_offset(offset)?;
    let line_offset = view.offset_of_line(line_num)?;

    let char_offset = count_utf16(&(view.get_line(line_num)?[0..(offset - line_offset)]));

    Ok(Position { line: line_num as u64, character: char_offset as u64 })
}

pub(crate) fn offset_of_position<C: Cache>(
    view: &mut View<C>,
    position: Position,
) -> Result<usize, PluginLibError> {
    let line_offset = view.offset_of_line(position.line as usize);

    let mut cur_len_utf16 = 0;
    let mut cur_len_utf8 = 0;

    for u in view.get_line(position.line as usize)?.chars() {
        if cur_len_utf16 >= (position.character as usize) {
            break;
        }
        cur_len_utf16 += u.len_utf16();
        cur_len_utf8 += u.len_utf8();
    }

    Ok(cur_len_utf8 + line_offset?)
}

pub(crate) fn core_range_from_range<C: Cache>(
    view: &mut View<C>,
    range: Range,
) -> Result<CoreRange, PluginLibError> {
    Ok(CoreRange {
        start: offset_of_position(view, range.start)?,
        end: offset_of_position(view, range.end)?,
    })
}

pub(crate) fn core_hover_from_hover<C: Cache>(
    view: &mut View<C>,
    hover: Hover,
) -> Result<CoreHover, LanguageResponseError> {
    Ok(CoreHover {
        content: markdown_from_hover_contents(hover.contents)?,
        range: match hover.range {
            Some(range) => Some(core_range_from_range(view, range)?),
            None => None,
        },
    })
}
