// Copyright 2016 Google Inc. All rights reserved.
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

//! A syntax highlighting plugin based on syntect.

extern crate syntect;
extern crate xi_rpc;
extern crate serde_json;

#[macro_use]
mod macros;

mod plugin_base;
mod caching_plugin;

use caching_plugin::{PluginCtx, SpansBuilder};

use syntect::parsing::{ParseState, ScopeStack, SyntaxSet};
use syntect::highlighting::{Color, FontStyle, Highlighter, HighlightIterator, HighlightState,
    Style, ThemeSet};

fn color_to_rgba(color: Color) -> u32 {
    ((color.a as u32) << 24) | ((color.r as u32) << 16) | ((color.g as u32) << 8) | (color.r as u32)
}

fn font_style_to_u8(fs: FontStyle) -> u8 {
    fs.bits()
}

fn add_style_span(builder: &mut SpansBuilder, style: Style, start: usize, end: usize) {
    builder.add_style_span(start, end,
        color_to_rgba(style.foreground), font_style_to_u8(style.font_style));
}

struct Sets {
    ss: SyntaxSet,
    ts: ThemeSet,
}

struct PluginState<'a> {
    sets: &'a Sets,
    line_num: usize,
    offset: usize,
    parse_state: Option<ParseState>,
    highlighter: Option<Highlighter<'a>>,
    hstate: Option<HighlightState>,
    spans_start: usize,
    builder: Option<SpansBuilder>,
}

impl<'a> PluginState<'a> {
    pub fn new(sets: &'a Sets) -> Self {
        PluginState {
            sets: sets,
            line_num: 0,
            offset: 0,
            parse_state: None,
            highlighter: None,
            hstate: None,
            spans_start: 0,
            builder: None,
        }
    }

    // Return true if there's more to do.
    fn highlight_one_line(&mut self, ctx: &mut PluginCtx) -> bool {
        let line = ctx.get_line(self.line_num);
        if let Err(err) = line {
            print_err!("Error: {:?}", err);
            return false;
        }
        let line = line.unwrap();
        if line.is_none() {
            return false;
        }
        let line = line.unwrap();
        let ops = self.parse_state.as_mut().unwrap().parse_line(&line);
        if self.builder.is_none() {
            self.spans_start = self.offset;
            self.builder = Some(SpansBuilder::new());
        }
        let iter = HighlightIterator::new(self.hstate.as_mut().unwrap(), &ops, &line,
            self.highlighter.as_ref().unwrap());
        let mut ix = 0;
        for (style, str_slice) in iter {
            let start = self.offset - self.spans_start + ix;
            let end = start + str_slice.len();
            add_style_span(self.builder.as_mut().unwrap(), style, start, end);
            ix += str_slice.len();
        }
        self.line_num += 1;
        self.offset += line.len();
        true
    }

    fn flush_spans(&mut self, ctx: &mut PluginCtx) {
        if let Some(builder) = self.builder.take() {
            ctx.set_fg_spans(self.spans_start, self.offset - self.spans_start, builder.build());
        }
    }

    fn do_highlighting(&mut self, mut ctx: PluginCtx) {
        let syntax = self.sets.ss.find_syntax_by_extension("rs")
            .unwrap_or_else(|| self.sets.ss.find_syntax_plain_text());
        self.parse_state = Some(ParseState::new(syntax));
        let theme = &self.sets.ts.themes["InspiredGitHub"];
        self.highlighter = Some(Highlighter::new(theme));
        self.hstate = Some(HighlightState::new(self.highlighter.as_ref().unwrap(),
            ScopeStack::new()));
        self.line_num = 0;
        self.offset = 0;
        ctx.schedule_idle(0);
    }
}

const LINES_PER_RPC: usize = 50;

impl<'a> caching_plugin::Handler for PluginState<'a> {
    fn init_buf(&mut self, ctx: PluginCtx, _buf_size: usize) {
        self.do_highlighting(ctx);
    }

    fn update(&mut self, ctx: PluginCtx) {
        self.do_highlighting(ctx);
    }

    fn idle(&mut self, mut ctx: PluginCtx, _token: usize) {
        print_err!("idle task at line {}", self.line_num);
        for _ in 0..LINES_PER_RPC {
            if !self.highlight_one_line(&mut ctx) {
                self.flush_spans(&mut ctx);
                return;
            }
            if ctx.request_is_pending() {
                print_err!("request pending at line {}", self.line_num);
                break;
            }
        }
        self.flush_spans(&mut ctx);
        ctx.schedule_idle(0);
    }
}

fn main() {
    let sets = Sets {
        ss: SyntaxSet::load_defaults_newlines(),
        ts: ThemeSet::load_defaults(),
    };
    let mut state = PluginState::new(&sets);

    caching_plugin::mainloop(&mut state);
}
