// Copyright 2016 The xi-editor Authors.
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
extern crate xi_core_lib as xi_core;
extern crate xi_plugin_lib;
extern crate xi_rope;
extern crate xi_trace;

mod stackmap;

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::MutexGuard;

use xi_core::plugin_rpc::ScopeSpan;
use xi_core::{ConfigTable, LanguageId, ViewId};
use xi_plugin_lib::{mainloop, Cache, Plugin, StateCache, View};
use xi_rope::delta::Builder as EditBuilder;
use xi_rope::interval::Interval;
use xi_rope::rope::RopeDelta;
use xi_trace::{trace, trace_block};

use stackmap::{LookupResult, StackMap};
use syntect::parsing::{ParseState, ScopeRepository, ScopeStack, SyntaxSet, SCOPE_REPO};

const LINES_PER_RPC: usize = 10;
const INDENTATION_PRIORITY: u64 = 100;

/// The state for syntax highlighting of one file.
struct PluginState {
    stack_idents: StackMap,
    offset: usize,
    initial_state: LineState,
    spans_start: usize,
    // unflushed spans
    spans: Vec<ScopeSpan>,
    new_scopes: Vec<Vec<String>>,
}

type LockedRepo = MutexGuard<'static, ScopeRepository>;

/// The syntax highlighting state corresponding to the beginning of a line
/// (as stored in the state cache).
// Note: this needs to be option because the caching layer relies on Default.
// We can't implement that because the actual initial state depends on the
// syntax. There are other ways to handle this, but this will do for now.
type LineState = Option<(ParseState, ScopeStack)>;

/// The state of syntax highlighting for a collection of buffers.
struct Syntect<'a> {
    view_state: HashMap<ViewId, PluginState>,
    syntax_set: &'a SyntaxSet,
}

impl PluginState {
    fn new() -> Self {
        PluginState {
            stack_idents: StackMap::default(),
            offset: 0,
            initial_state: None,
            spans_start: 0,
            spans: Vec::new(),
            new_scopes: Vec::new(),
        }
    }

    // compute syntax for one line, also accumulating the style spans
    fn compute_syntax(&mut self, line: &str, state: LineState) -> LineState {
        let (mut parse_state, mut scope_state) =
            state.or_else(|| self.initial_state.clone()).unwrap();
        let ops = parse_state.parse_line(&line);

        let mut prev_cursor = 0;
        let repo = SCOPE_REPO.lock().unwrap();
        for (cursor, batch) in ops {
            if scope_state.len() > 0 {
                let scope_id = self.identifier_for_stack(&scope_state, &repo);
                let start = self.offset - self.spans_start + prev_cursor;
                let end = start + (cursor - prev_cursor);
                if start != end {
                    let span = ScopeSpan { start, end, scope_id };
                    self.spans.push(span);
                }
            }
            prev_cursor = cursor;
            scope_state.apply(&batch);
        }
        // add span for final state
        let start = self.offset - self.spans_start + prev_cursor;
        let end = start + (line.len() - prev_cursor);
        let scope_id = self.identifier_for_stack(&scope_state, &repo);
        let span = ScopeSpan { start, end, scope_id };
        self.spans.push(span);
        Some((parse_state, scope_state))
    }

    /// Returns the unique identifier for this `ScopeStack`. We use identifiers
    /// so we aren't constantly sending long stack names to the peer.
    fn identifier_for_stack(&mut self, stack: &ScopeStack, repo: &LockedRepo) -> u32 {
        let identifier = self.stack_idents.get_value(stack.as_slice());
        match identifier {
            LookupResult::Existing(id) => id,
            LookupResult::New(id) => {
                let stack_strings =
                    stack.as_slice().iter().map(|slice| repo.to_string(*slice)).collect::<Vec<_>>();
                self.new_scopes.push(stack_strings);
                id
            }
        }
    }

    #[allow(unused)]
    // Return true if there's any more work to be done.
    fn highlight_one_line(&mut self, ctx: &mut MyView) -> bool {
        if let Some(line_num) = ctx.get_frontier() {
            let (line_num, offset, state) = ctx.get_prev(line_num);
            if offset != self.offset {
                self.flush_spans(ctx);
                self.offset = offset;
                self.spans_start = offset;
            }
            let new_frontier = match ctx.get_line(line_num) {
                Ok("") => None,
                Ok(s) => {
                    let new_state = self.compute_syntax(s, state);
                    self.offset += s.len();
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
                if let Some(old_state) = ctx.get(new_line_num) {
                    converged = old_state.as_ref().unwrap().0 == new_state.as_ref().unwrap().0;
                }
            }
            if !converged {
                if let Some((new_state, new_line_num)) = new_frontier {
                    ctx.set(new_line_num, new_state);
                    ctx.update_frontier(new_line_num);
                    return true;
                }
            }
            ctx.close_frontier();
        }
        false
    }

    fn flush_spans(&mut self, ctx: &mut MyView) {
        let _t = trace_block("PluginState::flush_spans", &["syntect"]);
        if !self.new_scopes.is_empty() {
            ctx.add_scopes(&self.new_scopes);
            self.new_scopes.clear();
        }
        if self.spans_start != self.offset {
            ctx.update_spans(self.spans_start, self.offset - self.spans_start, &self.spans);
            self.spans.clear();
        }
        self.spans_start = self.offset;
    }
}

type MyView = View<StateCache<LineState>>;

impl<'a> Syntect<'a> {
    fn new(syntax_set: &'a SyntaxSet) -> Self {
        Syntect { view_state: HashMap::new(), syntax_set }
    }

    /// Wipes any existing state and starts highlighting with `syntax`.
    fn do_highlighting(&mut self, view: &mut MyView) {
        let initial_state = {
            let language_id = view.get_language_id();
            let syntax = self
                .syntax_set
                .find_syntax_by_name(language_id.as_ref())
                .unwrap_or(self.syntax_set.find_syntax_plain_text());
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

    /// Checks if a newline has been inserted, and if so inserts whitespace
    /// as necessary.
    fn do_indentation(&mut self, view: &mut MyView, start: usize, end: usize, text: &str) {
        let _t = trace_block("PluginState::do_indentation", &["syntect"]);
        // don't touch indentation if this is not a simple edit
        if end != start {
            return;
        }

        let line_ending = view.get_config().line_ending.clone();
        let is_newline = line_ending == text;

        if is_newline {
            let line_num = view.line_of_offset(start).unwrap();

            let use_spaces = view.get_config().translate_tabs_to_spaces;
            let tab_size = view.get_config().tab_size;
            let buf_size = view.get_buf_size();

            let result = if let Some(line) = view.get_line(line_num).ok() {
                // do not send update if last line is empty string (contains only line ending)
                if line == line_ending {
                    return;
                }

                let indent = self.indent_for_next_line(line, use_spaces, tab_size);
                let ix = start + text.len();
                let interval = Interval::new_closed_open(ix, ix);
                //TODO: view should have a `get_edit_builder` fn?
                let mut builder = EditBuilder::new(buf_size);
                builder.replace(interval, indent.into());

                let delta = builder.build();
                Some(delta)
            } else {
                None
            };

            if let Some(delta) = result {
                view.edit(delta, INDENTATION_PRIORITY, false, false, String::from("syntect"));
            }
        }
    }

    /// Returns the string which should be inserted after the newline
    /// to achieve the desired indentation level.
    fn indent_for_next_line<'b>(
        &self,
        prev_line: &'b str,
        use_spaces: bool,
        tab_size: usize,
    ) -> Cow<'b, str> {
        let leading_ws = prev_line
            .char_indices()
            .find(|&(_, c)| !c.is_whitespace())
            .or(prev_line.char_indices().last())
            .map(|(idx, _)| unsafe { prev_line.get_unchecked(0..idx) })
            .unwrap_or("");

        if self.increase_indentation(prev_line) {
            let indent_text =
                if use_spaces { &"                                    "[..tab_size] } else { "\t" };
            format!("{}{}", leading_ws, indent_text).into()
        } else {
            leading_ws.into()
        }
    }

    /// Checks if the indent level should be increased.
    fn increase_indentation(&self, prev_line: &str) -> bool {
        let trailing_char = prev_line.trim_right().chars().rev().next().unwrap_or(' ');
        // very naive heuristic for modifying indentation level.
        match trailing_char {
            '{' | ':' => true,
            _ => false,
        }
    }
}

impl<'a> Plugin for Syntect<'a> {
    type Cache = StateCache<LineState>;

    fn new_view(&mut self, view: &mut View<Self::Cache>) {
        let _t = trace_block("Syntect::new_view", &["syntect"]);
        let view_id = view.get_id();
        let state = PluginState::new();
        self.view_state.insert(view_id, state);
        self.do_highlighting(view);
    }

    fn did_close(&mut self, view: &View<Self::Cache>) {
        self.view_state.remove(&view.get_id());
    }

    fn did_save(&mut self, view: &mut View<Self::Cache>, _old: Option<&Path>) {
        let _t = trace_block("Syntect::did_save", &["syntect"]);
        self.do_highlighting(view);
    }

    fn config_changed(&mut self, _view: &mut View<Self::Cache>, _changes: &ConfigTable) {}

    fn language_changed(&mut self, view: &mut View<Self::Cache>, _old_lang: LanguageId) {
        self.do_highlighting(view);
    }

    fn update(
        &mut self,
        view: &mut View<Self::Cache>,
        delta: Option<&RopeDelta>,
        _edit_type: String,
        _author: String,
    ) {
        let _t = trace_block("Syntect::update", &["syntect"]);
        view.schedule_idle();
        let should_auto_indent = view.get_config().auto_indent;
        if !should_auto_indent {
            return;
        }
        if let Some(delta) = delta {
            let (iv, _) = delta.summary();
            if let Some(s) = delta.as_simple_insert() {
                let s: String = s.into();
                self.do_indentation(view, iv.start(), iv.end(), &s);
            }
        }
    }

    fn idle(&mut self, view: &mut View<Self::Cache>) {
        let state = self.view_state.get_mut(&view.get_id()).unwrap();
        for _ in 0..LINES_PER_RPC {
            if !state.highlight_one_line(view) {
                state.flush_spans(view);
                return;
            }
            if view.request_is_pending() {
                trace("yielding for request", &["syntect"]);
                break;
            }
        }
        state.flush_spans(view);
        view.schedule_idle();
    }
}

fn main() {
    let syntax_set = SyntaxSet::load_defaults_newlines();
    let mut state = Syntect::new(&syntax_set);
    mainloop(&mut state).unwrap();
}
