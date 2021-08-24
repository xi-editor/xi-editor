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

//! Calc start of a backspace delete interval
use xi_rope::{Cursor, Rope};

use crate::config::BufferItems;
use crate::line_offset::{LineOffset, LogicalLines};
use crate::selection::SelRegion;
use xi_unicode::*;

#[allow(clippy::cognitive_complexity)]
pub fn offset_for_delete_backwards(region: &SelRegion, text: &Rope, config: &BufferItems) -> usize {
    if !region.is_caret() {
        region.min()
    } else {
        // backspace deletes max(1, tab_size) contiguous spaces
        let (_, c) = LogicalLines.offset_to_line_col(text, region.start);

        let tab_off = c % config.tab_size;
        let tab_size = config.tab_size;
        let tab_size = if tab_off == 0 { tab_size } else { tab_off };
        let tab_start = region.start.saturating_sub(tab_size);
        let preceded_by_spaces =
            region.start > 0 && (tab_start..region.start).all(|i| text.byte_at(i) == b' ');
        if preceded_by_spaces && config.translate_tabs_to_spaces && config.use_tab_stops {
            tab_start
        } else {
            #[derive(PartialEq)]
            enum State {
                Start,
                Lf,
                BeforeKeycap,
                BeforeVsAndKeycap,
                BeforeEmojiModifier,
                BeforeVSAndEmojiModifier,
                BeforeVS,
                BeforeEmoji,
                BeforeZwj,
                BeforeVSAndZWJ,
                OddNumberedRIS,
                EvenNumberedRIS,
                InTagSequence,
                Finished,
            }
            let mut state = State::Start;
            let mut tmp_offset = region.end;

            let mut delete_code_point_count = 0;
            let mut last_seen_vs_code_point_count = 0;

            while state != State::Finished && tmp_offset > 0 {
                let mut cursor = Cursor::new(text, tmp_offset);
                let code_point = cursor.prev_codepoint().unwrap_or('0');

                tmp_offset = text.prev_codepoint_offset(tmp_offset).unwrap_or(0);

                match state {
                    State::Start => {
                        delete_code_point_count = 1;
                        if code_point == '\n' {
                            state = State::Lf;
                        } else if is_variation_selector(code_point) {
                            state = State::BeforeVS;
                        } else if code_point.is_regional_indicator_symbol() {
                            state = State::OddNumberedRIS;
                        } else if code_point.is_emoji_modifier() {
                            state = State::BeforeEmojiModifier;
                        } else if code_point.is_emoji_combining_enclosing_keycap() {
                            state = State::BeforeKeycap;
                        } else if code_point.is_emoji() {
                            state = State::BeforeEmoji;
                        } else if code_point.is_emoji_cancel_tag() {
                            state = State::InTagSequence;
                        } else {
                            state = State::Finished;
                        }
                    }
                    State::Lf => {
                        if code_point == '\r' {
                            delete_code_point_count += 1;
                        }
                        state = State::Finished;
                    }
                    State::OddNumberedRIS => {
                        if code_point.is_regional_indicator_symbol() {
                            delete_code_point_count += 1;
                            state = State::EvenNumberedRIS
                        } else {
                            state = State::Finished
                        }
                    }
                    State::EvenNumberedRIS => {
                        if code_point.is_regional_indicator_symbol() {
                            delete_code_point_count -= 1;
                            state = State::OddNumberedRIS;
                        } else {
                            state = State::Finished;
                        }
                    }
                    State::BeforeKeycap => {
                        if is_variation_selector(code_point) {
                            last_seen_vs_code_point_count = 1;
                            state = State::BeforeVsAndKeycap;
                        } else {
                            if is_keycap_base(code_point) {
                                delete_code_point_count += 1;
                            }
                            state = State::Finished;
                        }
                    }
                    State::BeforeVsAndKeycap => {
                        if is_keycap_base(code_point) {
                            delete_code_point_count += last_seen_vs_code_point_count + 1;
                        }
                        state = State::Finished;
                    }
                    State::BeforeEmojiModifier => {
                        if is_variation_selector(code_point) {
                            last_seen_vs_code_point_count = 1;
                            state = State::BeforeVSAndEmojiModifier;
                        } else {
                            if code_point.is_emoji_modifier_base() {
                                delete_code_point_count += 1;
                            }
                            state = State::Finished;
                        }
                    }
                    State::BeforeVSAndEmojiModifier => {
                        if code_point.is_emoji_modifier_base() {
                            delete_code_point_count += last_seen_vs_code_point_count + 1;
                        }
                        state = State::Finished;
                    }
                    State::BeforeVS => {
                        if code_point.is_emoji() {
                            delete_code_point_count += 1;
                            state = State::BeforeEmoji;
                        } else {
                            if !is_variation_selector(code_point) {
                                //TODO: UCharacter.getCombiningClass(codePoint) == 0
                                delete_code_point_count += 1;
                            }
                            state = State::Finished;
                        }
                    }
                    State::BeforeEmoji => {
                        if code_point.is_zwj() {
                            state = State::BeforeZwj;
                        } else {
                            state = State::Finished;
                        }
                    }
                    State::BeforeZwj => {
                        if code_point.is_emoji() {
                            delete_code_point_count += 2;
                            state = if code_point.is_emoji_modifier() {
                                State::BeforeEmojiModifier
                            } else {
                                State::BeforeEmoji
                            };
                        } else if is_variation_selector(code_point) {
                            last_seen_vs_code_point_count = 1;
                            state = State::BeforeVSAndZWJ;
                        } else {
                            state = State::Finished;
                        }
                    }
                    State::BeforeVSAndZWJ => {
                        if code_point.is_emoji() {
                            delete_code_point_count += last_seen_vs_code_point_count + 2;
                            last_seen_vs_code_point_count = 0;
                            state = State::BeforeEmoji;
                        } else {
                            state = State::Finished;
                        }
                    }
                    State::InTagSequence => {
                        if code_point.is_tag_spec_char() {
                            delete_code_point_count += 1;
                        } else if code_point.is_emoji() {
                            delete_code_point_count += 1;
                            state = State::Finished;
                        } else {
                            delete_code_point_count = 1;
                            state = State::Finished;
                        }
                    }
                    State::Finished => {
                        break;
                    }
                }
            }

            let mut start = region.end;
            while delete_code_point_count > 0 {
                start = text.prev_codepoint_offset(start).unwrap_or(0);
                delete_code_point_count -= 1;
            }
            start
        }
    }
}
