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

mod plugin_base;

use plugin_base::{PluginRequest, PluginPeer, SpansBuilder};

use syntect::parsing::{ParseState, ScopeStack, SyntaxSet};
use syntect::highlighting::{Color, FontStyle, Highlighter, HighlightIterator, HighlightState,
    Style, ThemeSet};

// TODO: avoid duplicating this in every crate
macro_rules! print_err {
    ($($arg:tt)*) => (
        {
            use std::io::prelude::*;
            if let Err(e) = write!(&mut ::std::io::stderr(), "{}\n", format_args!($($arg)*)) {
                panic!("Failed to write to stderr.\
                    \nOriginal error output: {}\
                    \nSecondary error writing to stderr: {}", format!($($arg)*), e);
            }
        }
    )
}

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

struct PluginState {
    ss: SyntaxSet,
    ts: ThemeSet,
}

impl PluginState {
    pub fn new() -> Self {
        PluginState {
            ss: SyntaxSet::load_defaults_newlines(),
            ts: ThemeSet::load_defaults(),
        }
    }
}

fn do_highlighting(peer: &PluginPeer, state: &PluginState) {
    let syntax = state.ss.find_syntax_by_extension("rs")
        .unwrap_or_else(|| state.ss.find_syntax_plain_text());
    let mut parse_state = ParseState::new(syntax);
    let theme = &state.ts.themes["InspiredGitHub"];
    let highlighter = Highlighter::new(theme);
    let mut hstate = HighlightState::new(&highlighter, ScopeStack::new());

    let n_lines = peer.n_lines();
    if let Err(err) = n_lines {
        // TODO: maybe try to report the error back to the peer
        print_err!("Error: {:?}", err);
        return;
    }
    let n_lines = n_lines.unwrap();
    for i in 0..n_lines {
        let line = peer.get_line(i);
        if let Err(err) = line {
            // TODO: as above
            print_err!("Error: {:?}", err);
            return;
        }
        let line = line.unwrap();
        let ops = parse_state.parse_line(&line);
        let mut builder = SpansBuilder::new();
        let iter = HighlightIterator::new(&mut hstate, &ops, &line, &highlighter);
        let mut ix = 0;
        for (style, str_slice) in iter {
            let start = ix;
            let end = ix + str_slice.len();
            add_style_span(&mut builder, style, start, end);
            ix = end;
        }
        peer.set_line_fg_spans(i, builder.build());
    }
}

fn main() {
    let state = PluginState::new();

    plugin_base::mainloop(|req, peer| {
        match *req {
            PluginRequest::Ping => {
                print_err!("got ping");
                None
            }
            PluginRequest::PingFromEditor => {
                print_err!("got ping from editor");
                do_highlighting(peer, &state);
                None
            }
        }
    });
}
