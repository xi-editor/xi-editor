// Copyright 2018 Google LLC
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

use lsp_types::*;
use types::DefinitionResult;
use xi_plugin_lib::{
    Cache, Definition as CoreDefinition, Error as PluginLibError, Hover as CoreHover,
    LanguageResponseError, Location as CoreLocation, Position as CorePosition, Range as CoreRange,
    View,
};

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
            let res: Vec<String> = content.iter().map(|c| marked_string_to_string(c)).collect();
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

    Ok(Position {
        line: line_num as u64,
        character: char_offset as u64,
    })
}

pub(crate) fn lsp_position_from_core_position<C: Cache>(
    view: &mut View<C>,
    position: CorePosition,
) -> Result<Position, PluginLibError> {
    match position {
        CorePosition::Utf8LineChar { line, character } => {
            let line_text = view.get_line(line)?;
            let char_offset: usize = line_text[0..character].chars().map(char::len_utf16).sum();

            Ok(Position {
                line: line as u64,
                character: char_offset as u64,
            })
        }
        CorePosition::Utf16LineChar { line, character } => Ok(Position {
            line: line as u64,
            character: character as u64,
        }),
        CorePosition::Utf8Offset { offset } => get_position_of_offset(view, offset),
    }
}

pub(crate) fn core_position_from_position(position: Position) -> CorePosition {
    CorePosition::Utf16LineChar {
        line: position.line as usize,
        character: position.character as usize,
    }
}

pub(crate) fn core_range_from_range(range: Range) -> CoreRange {
    CoreRange {
        start: core_position_from_position(range.start),
        end: core_position_from_position(range.end),
    }
}

pub(crate) fn core_location_from_location(location: &Location) -> CoreLocation {
    CoreLocation {
        path: location.uri.to_file_path().unwrap(),
        range: core_range_from_range(location.range),
    }
}

pub(crate) fn core_definition_from_definition(
    definition: DefinitionResult,
) -> Result<CoreDefinition, LanguageResponseError> {
    match definition {
        DefinitionResult::Location(location) => Ok(CoreDefinition {
            locations: vec![core_location_from_location(&location)],
        }),
        DefinitionResult::Locations(locations) => Ok(CoreDefinition {
            locations: locations.iter().map(core_location_from_location).collect(),
        }),
        DefinitionResult::Null => Err(LanguageResponseError::NullResponse),
    }
}

pub(crate) fn core_hover_from_hover(hover: Hover) -> Result<CoreHover, LanguageResponseError> {
    Ok(CoreHover {
        content: markdown_from_hover_contents(hover.contents)?,
        range: hover.range.map(|range| core_range_from_range(range)),
    })
}
