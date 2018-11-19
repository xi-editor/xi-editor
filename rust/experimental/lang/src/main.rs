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
extern crate xi_trace;

use xi_core_lib::{ConfigTable, LanguageId, plugins::rpc::ScopeSpan, ViewId};
use xi_plugin_lib::{mainloop, Plugin, StateCache, View};
use xi_rope::RopeDelta;
use xi_trace::{trace, trace_block, trace_payload};

use std::{collections::HashMap, env, path::Path};

use rust::{RustParser, StateEl};
use statestack::{HolderNewState, State};
use tracker::{ElementTracker, LookupResult};

mod peg;
mod rust;
mod tracker;
mod statestack;

const LINES_PER_RPC: usize = 50;

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
    type Cache = StateCache<State>;

    fn update(
        &mut self,
        view: &mut View<Self::Cache>,
        _delta: Option<&RopeDelta>,
        _edit_type: String,
        _author: String,
    ) {
        let _guard = trace_block("ExperimentalLang::update", &["experimental-lang"]);
        view.schedule_idle();
    }

    fn did_save(&mut self, view: &mut View<Self::Cache>, _old_path: Option<&Path>) {
        let _guard = trace_block("ExperimentalLang::did_save", &["experimental-lang"]);

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
        let _guard = trace_block("ExperimentalLang::new_view", &["experimental-lang"]);

        let view_id = view.get_id();
        let mut view_state = ViewState::new();

        view_state.do_highlighting(view);
        self.view_states.insert(view_id, view_state);
    }

    fn config_changed(&mut self, _view: &mut View<Self::Cache>, _changes: &ConfigTable) {}

    fn language_changed(&mut self, view: &mut View<<Self as Plugin>::Cache>, _old_lang: LanguageId) {
        let _guard = trace_block("ExperimentalLang::language_changed", &["experimental-lang"]);

        let view_id = view.get_id();
        if let Some(view_state) = self.view_states.get_mut(&view_id) {
            view_state.do_highlighting(view);
        }
    }

    fn idle(&mut self, view: &mut View<Self::Cache>) {
        let view_id = view.get_id();

        if let Some(view_state) = self.view_states.get_mut(&view_id) {
            for _ in 0..LINES_PER_RPC {
                if !view_state.highlight_one_line(view) {
                    view_state.flush_spans(view);
                    return;
                }

                if view.request_is_pending() {
                    trace("yielding for request", &["experimental-lang"]);
                    break;
                }
            }

            view_state.flush_spans(view);
            view.schedule_idle();
        }
    }
}

struct ViewState {
    parser: RustParser<HolderNewState<StateEl>>,
    tracker: ElementTracker,
    line_num: usize,
    offset: usize,
    initial_state: State,
    spans_start: usize,
    spans: Vec<ScopeSpan>,
    new_scopes: Vec<Vec<String>>,
}

impl ViewState {
    fn new() -> ViewState {
        ViewState {
            parser: RustParser::new(HolderNewState::new()),
            tracker: ElementTracker::default(),
            line_num: 0,
            offset: 0,
            initial_state: State::default(),
            spans_start: 0,
            spans: Vec::new(),
            new_scopes: Vec::new(),
        }
    }

    fn can_highlight(&self, language_id: &LanguageId) -> bool {
        language_id.as_ref() == "Rust"
    }

    fn do_highlighting(&mut self, view: &mut View<StateCache<State>>) {
        if !self.can_highlight(view.get_language_id()) {
            trace("unsupported language", &["experimental-lang"]);
            return;
        }

        self.line_num = 0;
        self.offset = 0;
        self.initial_state = State::default();
        self.spans = Vec::new();
        self.new_scopes = Vec::new();
        view.schedule_idle();
    }

    fn highlight_one_line(&mut self, view: &mut View<StateCache<State>>) -> bool {
        if let Some(line_num) = view.get_frontier() {
            let (line_num, offset, _state) = view.get_prev(line_num);

            if offset != self.offset {
                self.flush_spans(view);
                self.offset = offset;
                self.spans_start = offset;
            }

            let new_frontier = match view.get_line(line_num) {
                Ok("") => None,
                Ok(line) => {
                    let new_state = self.compute_syntax(line);
                    self.offset += line.len();

                    if line.as_bytes().last() == Some(&b'\n') {
                        Some((new_state, line_num + 1))
                    } else {
                        None
                    }
                }
                Err(_) => None,
            };

            let mut converged = false;
            if let Some((ref new_state, new_line_num)) = new_frontier {
                if let Some(old_state) = view.get(new_line_num) {
                    converged = old_state == new_state;
                }
            }

            if !converged {
                if let Some((new_state, new_line_num)) = new_frontier {
                    view.set(new_line_num, new_state);
                    view.update_frontier(new_line_num);
                    return true;
                }
            }

            view.close_frontier();
        }

        false
    }

    fn compute_syntax(&mut self, line: &str) -> State {
        let mut i = 0;

        let mut state = self.initial_state;
        while i < line.len() {
            let (prevlen, s0, len, s1) = self.parser.parse(&line[i..], state);

            if prevlen > 0 {
                // TODO: maybe make an iterator to avoid this duplication
                let element = self.parser.get_new_state().get_element(self.initial_state);
                if let Some(ref element) = element {
                    let scope_id = match self.tracker.lookup(element) {
                        LookupResult::Existing(id) => id,
                        LookupResult::New(id) => {
                            self.new_scopes.push(element.as_scopes());
                            id
                        }
                    };

                    let start = self.offset - self.spans_start + i;
                    let end = start + prevlen;

                    let span = ScopeSpan { start, end, scope_id };
                    self.spans.push(span);
                }

                i += prevlen;
            }

            let element = self.parser.get_new_state().get_element(s0);
            if let Some(ref element) = element {
                let scope_id = match self.tracker.lookup(element) {
                    LookupResult::Existing(id) => id,
                    LookupResult::New(id) => {
                        self.new_scopes.push(element.as_scopes());
                        id
                    }
                };

                let start = self.offset - self.spans_start + i;
                let end = start + len;

                let span = ScopeSpan { start, end, scope_id };
                self.spans.push(span);
            }

            i += len;
            state = s1;
        }

        state
    }

    #[allow(unused)]
    fn identifier_for_element(&mut self, element: &StateEl) -> u32 {
        match self.tracker.lookup(element) {
            LookupResult::Existing(id) => id,
            LookupResult::New(id) => {
                self.new_scopes.push(element.as_scopes());
                id
            }
        }
    }

    fn flush_spans(&mut self, view: &mut View<StateCache<State>>) {
        if !self.new_scopes.is_empty() {
            trace_payload("flushing scopes", &["experimental-lang"], format!("flushing scopes: {:?}", self.new_scopes));
            eprintln!("flushing scopes: {:?}", self.new_scopes);
            view.add_scopes(&self.new_scopes);
            self.new_scopes.clear();
        }

        if self.spans_start != self.offset {
            trace_payload("flushing spans", &["experimental-lang"], format!("flushing spans: {:?}", self.spans));
            eprintln!("flushing spans: {:?}", self.spans);
            view.update_spans(self.spans_start, self.offset - self.spans_start, &self.spans);
            self.spans.clear();
        }

        self.spans_start = self.offset;
    }
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