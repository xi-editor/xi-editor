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
use std::collections::HashMap;
use std::path::Path;

use syntect::parsing::{ParseState, ScopeStack, SyntaxSet, SyntaxDefinition};

use xi_core::{ViewIdentifier, ConfigTable};
use xi_core::plugin_rpc::PluginEdit;
use xi_rope::rope::RopeDelta;
use xi_plugin_lib::global::{Cache, Plugin, View, mainloop};
use xi_plugin_lib::state_cache::StateCache;

use local::{PluginState, State as LineState, LINES_PER_RPC};

/// The state of syntax highlighting for a collection of buffers.
struct Syntect<'a> {
    view_state: HashMap<ViewIdentifier, PluginState<'a>>,
    syntax_set: &'a SyntaxSet,
}

impl<'a> Syntect<'a> {
    fn new(syntax_set: &'a SyntaxSet) -> Self {
        Syntect {
            view_state: HashMap::new(),
            syntax_set: syntax_set,
        }
    }
}

impl<'a> Plugin for Syntect<'a> {
    type Cache = StateCache<LineState>;

    fn new_view(&mut self, view: &mut View<Self::Cache>) {
        eprintln!("added view {:?}", view.get_id());
        let view_id = view.get_id();
        let state = PluginState::new(self.syntax_set);
        self.view_state.insert(view_id, state);
        self.do_highlighting(view);
    }

    fn did_close(&mut self, view: &View<Self::Cache>) {
        eprintln!("removed view {:?}", view.get_id());
        self.view_state.remove(&view.get_id());
    }

    fn did_save(&mut self, view: &mut View<Self::Cache>, _old: Option<&Path>) {
        self.do_highlighting(view);
    }

    fn config_changed(&mut self, _view: &mut View<Self::Cache>, _changes: &ConfigTable) {
    }

    fn update(&mut self, view: &mut View<Self::Cache>, _delta: Option<&RopeDelta>,
              _edit_type: String, _author: String) -> Option<PluginEdit> {
        view.schedule_idle();
        None
    }

    fn idle(&mut self, view: &mut View<Self::Cache>) {
        let state = self.view_state.get_mut(&view.get_id()).unwrap();
        for _ in 0..LINES_PER_RPC {
            if !highlight_one_line(view, state) {
                flush_spans(view, state);
                return;
            }
            if view.request_is_pending() {
                eprintln!("request pending for {:?} at offset {}",
                          view.get_id(), state.offset);
                break;
            }
        }
        flush_spans(view, state);
        view.schedule_idle();
    }
}

type MyView = View<StateCache<LineState>>;

impl<'a> Syntect<'a> {
    /// Wipes any existing state and starts highlighting with `syntax`.
    fn do_highlighting(&mut self, view: &mut MyView) {
        let initial_state = {
            let syntax = self.guess_syntax(view.get_path());
            Some((ParseState::new(syntax), ScopeStack::new()))
        };

        let state = self.view_state.get_mut(&view.get_id()).unwrap();
        state.initial_state = initial_state;
        state.spans = Vec::new();
        state.new_scopes = Vec::new();
        state.offset = 0;
        state.spans_start = 0;
        view.get_cache().clear();
        view.schedule_idle();
    }


    fn guess_syntax(&'a self, path: Option<&Path>) -> &'a SyntaxDefinition {
        match path {
            Some(path) => self.syntax_set.find_syntax_for_file(path).unwrap()
                .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text()),
            None => self.syntax_set.find_syntax_plain_text(),
        }
    }
}

// getting around the borrow checker. If this is working we should adapt local::PluginState
// to take a trait argument that can be a `View` or a `PluginCtx`.
/// Highlight a single line, returning a bool indicating whether or
/// not there is more work to be done.
fn highlight_one_line(view: &mut MyView, view_state: &mut PluginState) -> bool {
    if let Some(line_num) = view.get_frontier() {
        let (line_num, offset, state) = view.get_prev(line_num);
        if offset != view_state.offset {
            flush_spans(view, view_state);
            view_state.offset = offset;
            view_state.spans_start = offset;
        }
        let new_frontier = match view.get_line(line_num) {
            Ok("") => None,
            Ok(s) => {
                let new_state = view_state.compute_syntax(s, state);
                view_state.offset += s.len();
                if s.as_bytes().last() == Some(&b'\n') {
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
                converged = old_state.as_ref().unwrap().0 == new_state.as_ref().unwrap().0;
            }
        }
        if !converged {
            if let Some((new_state, new_line_num)) = new_frontier {
                view.set(new_line_num, new_state);
                view.update_frontier(new_line_num);
                return true;
            }
        }
        view.get_cache().close_frontier();
    }
    false
}

fn flush_spans(view: &mut MyView, view_state: &mut PluginState) {
    if !view_state.new_scopes.is_empty() {
        view.add_scopes(&view_state.new_scopes);
        view_state.new_scopes.clear();
    }
    if view_state.spans_start != view_state.offset {
        view.update_spans(view_state.spans_start, view_state.offset - view_state.spans_start,
                          &view_state.spans);
        view_state.spans.clear();
    }
    view_state.spans_start = view_state.offset;
}

#[allow(dead_code)]
pub(crate) fn main() {
    let syntax_set = SyntaxSet::load_defaults_newlines();
    let mut state = Syntect::new(&syntax_set);
    mainloop(&mut state).unwrap();
}
