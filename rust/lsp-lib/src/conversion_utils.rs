use lsp_types::*;
use types::DefinitionResult;
use xi_plugin_lib::HoverResult;
use xi_plugin_lib::LanguageResponseError;
use xi_plugin_lib::{
    Cache, DefinitionResult as CoreDefinitionResult, Error as PluginLibError,
    Location as CoreLocation, Position as CorePosition, Range as CoreRange, View,
};

pub fn marked_string_to_string(marked_string: &MarkedString) -> String {
    match *marked_string {
        MarkedString::String(ref text) => text.to_owned(),
        MarkedString::LanguageString(ref d) => format!("```{}\n{}\n```", d.language, d.value),
    }
}

pub fn markdown_from_hover_contents(
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
pub fn count_utf16(s: &str) -> usize {
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
pub fn get_position_of_offset<C: Cache>(
    view: &mut View<C>,
    offset: usize,
) -> Result<Position, PluginLibError> {
    let line_num = view.line_of_offset(offset)?;
    let line_offset = view.offset_of_line(line_num)?;

    let char_offset: usize = count_utf16(&(view.get_line(line_num)?[0..(offset - line_offset)]));

    Ok(Position {
        line: line_num as u64,
        character: char_offset as u64,
    })
}

pub fn lsp_position_from_core_position<C: Cache>(
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

pub fn core_position_from_position(position: Position) -> CorePosition {
    CorePosition::Utf16LineChar {
        line: position.line as usize,
        character: position.character as usize,
    }
}

pub fn core_range_from_range(range: Range) -> CoreRange {
    CoreRange {
        start: core_position_from_position(range.start),
        end: core_position_from_position(range.end),
    }
}

pub fn core_location_from_location(location: &Location) -> CoreLocation {
    CoreLocation {
        path: location.uri.to_file_path().unwrap(),
        range: core_range_from_range(location.range),
    }
}

pub fn core_definition_from_definition(
    definition: DefinitionResult,
) -> Result<CoreDefinitionResult, LanguageResponseError> {
    match definition {
        DefinitionResult::Location(location) => Ok(CoreDefinitionResult::Location {
            location: core_location_from_location(&location),
        }),
        DefinitionResult::Locations(locations) => Ok(CoreDefinitionResult::Locations {
            locations: locations
                .iter()
                .map(|l| core_location_from_location(l))
                .collect(),
        }),
        DefinitionResult::Null => Err(LanguageResponseError::NullResponse),
    }
}

pub fn core_hover_result_from_hover(hover: Hover) -> Result<HoverResult, LanguageResponseError> {
    Ok(HoverResult {
        content: markdown_from_hover_contents(hover.contents)?,
        range: hover.range.map(|range| core_range_from_range(range)),
    })
}
