// Copyright 2017 Google Inc. All rights reserved.
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

//! A language syntax coloring and indentation plugin for xi-editor.

#[macro_use]
extern crate xi_plugin_lib;

use xi_plugin_lib::caching_plugin::{self, PluginCtx, SpansBuilder};

use std::env;

mod rust;
mod colorize;
mod statestack;
mod peg;

use rust::RustColorize;
use statestack::State;
use colorize::{Style, StyleNewState, Colorize};

fn add_style_span(builder: &mut SpansBuilder, style: &Style, start: usize, end: usize) {
    builder.add_style_span(start, end,
        style.fg_color, style.font);
}

struct PluginState {
    colorize: RustColorize<StyleNewState<fn(&mut Style, &rust::StateEl)>>,
    line_num: usize,
    offset: usize,
    state: State,
    spans_start: usize,
    builder: Option<SpansBuilder>,
}

impl PluginState {
    fn new() -> PluginState {
        PluginState {
            colorize: RustColorize::new(StyleNewState::new(rust::to_style)),
            line_num: 0,
            offset: 0,
            state: State::default(),
            spans_start: 0,
            builder: None,
        }
    }

    // Return true if there's more to do.
    fn highlight_one_line(&mut self, ctx: &mut PluginCtx) -> bool {
        let line = ctx.get_line(self.line_num);
        if let Err(err) = line {
            eprintln!("Error: {:?}", err);
            return false;
        }
        let line = line.unwrap();
        if line.is_none() {
            return false;
        }

        let line = line.unwrap();
        if self.builder.is_none() {
            self.spans_start = self.offset;
            self.builder = Some(SpansBuilder::new());
        }

        let mut i = 0;
        while i < line.len() {
            let (prevlen, s0, len, s1) = self.colorize.colorize(&line[i..], self.state);
            if prevlen > 0 {
                // TODO: maybe make an iterator to avoid this duplication
                let style = self.colorize.get_new_state().get_style(self.state);
                let start = self.offset - self.spans_start + i;
                let end = start + prevlen;
                add_style_span(self.builder.as_mut().unwrap(), style, start, end);
                i += prevlen;
            }
            let style = self.colorize.get_new_state().get_style(s0);
            let start = self.offset - self.spans_start + i;
            let end = start + len;
            add_style_span(self.builder.as_mut().unwrap(), style, start, end);
            i += len;
            self.state = s1;
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
        self.line_num = 0;
        self.offset = 0;
        self.state = State::default();
        ctx.schedule_idle(0);
    }
}

const LINES_PER_RPC: usize = 50;

impl caching_plugin::Handler for PluginState {
    fn initialize(&mut self, ctx: PluginCtx, _buf_size: usize) {
        self.do_highlighting(ctx);
    }

    fn update(&mut self, ctx: PluginCtx) {
        self.do_highlighting(ctx);
    }

    fn idle(&mut self, mut ctx: PluginCtx, _token: usize) {
        eprintln!("idle task at line {}", self.line_num);
        for _ in 0..LINES_PER_RPC {
            if !self.highlight_one_line(&mut ctx) {
                self.flush_spans(&mut ctx);
                return;
            }
            if ctx.request_is_pending() {
                eprintln!("request pending at line {}", self.line_num);
                break;
            }
        }
        self.flush_spans(&mut ctx);
        ctx.schedule_idle(0);
    }
}

fn xi_plugin_main() {
    let mut state = PluginState::new();
    caching_plugin::mainloop(&mut state);
}

fn main() {
    if let Some(ref s) = env::args().skip(1).next() {
        if s == "test" {
            rust::test();
            return;
        }
    }
    xi_plugin_main();
}
