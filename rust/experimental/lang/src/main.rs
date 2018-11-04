// Copyright 2017 The xi-editor Authors.
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

extern crate xi_core_lib;
extern crate xi_rope;

use xi_core_lib::{ConfigTable, ViewId};
use xi_plugin_lib::{Cache, ChunkCache, CoreProxy, mainloop, Plugin, View};
use xi_rope::{Interval, RopeDelta, spans::SpansBuilder};

use std::{env, path::Path, collections::HashMap};

use colorize::{Style, StyleNewState, Colorize};
use rust::RustColorize;
use statestack::State;

mod colorize;
mod peg;
mod rust;
mod statestack;

const LINES_PER_RPC: usize = 50;

// Possibly swap this out for something more appropriate down the line
type StyleCache = ChunkCache;

struct LangPlugin {
    view_states: HashMap<ViewId, ViewState>
}

impl LangPlugin {
    fn new() -> LangPlugin {
        LangPlugin {
            view_states: HashMap::new()
        }
    }
}

impl Plugin for LangPlugin {
    type Cache = StyleCache;

    fn initialize(&mut self, core: CoreProxy) {
        //self.do_highlighting(ctx);
    }

    fn update(
        &mut self,
        view: &mut View<Self::Cache>,
        delta: Option<&RopeDelta>,
        edit_type: String,
        author: String,
    ) {
        let view_id = view.get_id();

        if let Some(view_state) = self.view_states.get_mut(&view_id) {
            view_state.do_highlighting(view);
        }
    }

    fn did_save(&mut self, view: &mut View<Self::Cache>, old_path: Option<&Path>) {
        let view_id = view.get_id();

        if let Some(view_state) = self.view_states.get_mut(&view_id) {
            view_state.do_highlighting(view);
        }
    }

    fn did_close(&mut self, view: &View<Self::Cache>) {
        let view_id = view.get_id();
        self.view_states.remove(&view_id);
    }

    fn new_view(&mut self, view: &mut View<Self::Cache>) {
        let view_id = view.get_id();
        let mut view_state = ViewState::new();
        view_state.do_highlighting(view);
        self.view_states.insert(view_id, view_state);
    }

    fn config_changed(&mut self, view: &mut View<Self::Cache>, changes: &ConfigTable) {}

    fn idle(&mut self, view: &mut View<Self::Cache>) {
        let view_id = view.get_id();

        if let Some(view_state) = self.view_states.get_mut(&view_id) {
            eprintln!("idle task at line {}", view_state.line_num);
            for _ in 0..LINES_PER_RPC {
                if !view_state.highlight_one_line(view) {
                    view_state.flush_spans(view);
                    return;
                }

                if view.request_is_pending() {
                    eprintln!("request pending at line {}", view_state.line_num);
                    break;
                }
            }

            view_state.flush_spans(view);
            view.schedule_idle();
        }
    }
}

struct ViewState {
    colorize: RustColorize<StyleNewState<fn(&mut Style, &rust::StateEl)>>,
    line_num: usize,
    offset: usize,
    state: State,
    spans_start: usize,
    builder: Option<SpansBuilder<Style>>,
}

impl ViewState {
    fn new() -> ViewState {
        ViewState {
            colorize: RustColorize::new(StyleNewState::new(rust::to_style)),
            line_num: 0,
            offset: 0,
            state: State::default(),
            spans_start: 0,
            builder: None,
        }
    }

    fn do_highlighting(&mut self, view: &mut View<StyleCache>) {
        self.line_num = 0;
        self.offset = 0;
        self.state = State::default();
        view.schedule_idle();
    }

    // Return true if there's more to do.
    fn highlight_one_line(&mut self, view: &mut View<StyleCache>) -> bool {
        let line = view.get_line(self.line_num);
        if let Err(err) = line {
            eprintln!("Error: {:?}", err);
            return false;
        }

        let line = line.unwrap();

        if self.builder.is_none() {
            self.spans_start = self.offset;
            self.builder = Some(SpansBuilder::new(line.len()));
        }

        let mut i = 0;
        while i < line.len() {
            let (prevlen, s0, len, s1) = self.colorize.colorize(&line[i..], self.state);

            if prevlen > 0 {
                // TODO: maybe make an iterator to avoid this duplication
                let style = self.colorize.get_new_state().get_style(self.state);
                let start = self.offset - self.spans_start + i;
                let end = start + prevlen;
                add_style_span(self.builder.as_mut().unwrap(), style.clone(), start, end);
                i += prevlen;
            }

            let style = self.colorize.get_new_state().get_style(s0);

            let start = self.offset - self.spans_start + i;
            let end = start + len;

            add_style_span(self.builder.as_mut().unwrap(), style.clone(), start, end);

            i += len;
            self.state = s1;
        }

        self.line_num += 1;
        self.offset += line.len();

        true
    }

    fn flush_spans(&mut self, view: &mut View<StyleCache>) {
        if let Some(builder) = self.builder.take() {
            let spans = builder.build();
            view.update_spans(self.spans_start, self.offset - self.spans_start, spans);
        }
    }
}

fn add_style_span(builder: &mut SpansBuilder<Style>, style: Style, start: usize, end: usize) {
    builder.add_span(Interval::new(start, end), style);
}

fn main() {
    if let Some(ref s) = env::args().skip(1).next() {
        if s == "test" {
            rust::test();
            return;
        }
    }

    let mut plugin = LangPlugin::new();
    mainloop(&mut plugin).unwrap()
}