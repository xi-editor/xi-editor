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

use std::collections::HashMap;
use std::path::Path;
use std::sync::MutexGuard;

use xi_core::plugin_rpc::ScopeSpan;
use xi_core::{ConfigTable, LanguageId, ViewId};
use xi_plugin_lib::{mainloop, Cache, Error, Plugin, StateCache, View};
use xi_rope::{Interval, RopeDelta};
use xi_trace::{trace, trace_block};

use syntect::parsing::{
    ParseState, ScopeRepository, ScopeStack, ScopedMetadata, SyntaxSet, SCOPE_REPO,
};

use stackmap::{LookupResult, StackMap};

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

    /// Compute syntax for one line, optionally also accumulating the style spans.
    ///
    /// NOTE: `accumulate_spans` should be true if we're doing syntax highlighting,
    /// and want to update the client. It should be `false` if we need syntax
    /// infromation for another purpose, such as auto-indent.
    fn compute_syntax(
        &mut self,
        line: &str,
        state: LineState,
        syntax_set: &SyntaxSet,
        accumulate_spans: bool,
    ) -> LineState {
        let (mut parse_state, mut scope_state) =
            state.or_else(|| self.initial_state.clone()).unwrap();
        let ops = parse_state.parse_line(&line, syntax_set);

        let mut prev_cursor = 0;
        let repo = SCOPE_REPO.lock().unwrap();
        for (cursor, batch) in ops {
            if !scope_state.is_empty() {
                let scope_id = self.identifier_for_stack(&scope_state, &repo);
                let start = self.offset - self.spans_start + prev_cursor;
                let end = start + (cursor - prev_cursor);
                if accumulate_spans && start != end {
                    let span = ScopeSpan { start, end, scope_id };
                    self.spans.push(span);
                }
            }
            prev_cursor = cursor;
            scope_state.apply(&batch);
        }

        if accumulate_spans {
            // add span for final state
            let start = self.offset - self.spans_start + prev_cursor;
            let end = start + (line.len() - prev_cursor);
            let scope_id = self.identifier_for_stack(&scope_state, &repo);
            let span = ScopeSpan { start, end, scope_id };
            self.spans.push(span);
        }
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

    // Return true if there's any more work to be done.
    fn highlight_one_line(&mut self, ctx: &mut MyView, syntax_set: &SyntaxSet) -> bool {
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
                    let new_state = self.compute_syntax(s, state, syntax_set, true);
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
                .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());
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

    /// Returns the metadata relevant to the given line. Computes the syntax
    /// for this line (during normal editing this is only likely for line 0) if
    /// necessary; in general reuses the syntax state calculated for highlighting.
    fn get_metadata(&mut self, view: &mut MyView, line: usize) -> Option<ScopedMetadata> {
        // we don't store the state for the first line, so recompute it
        let Syntect { view_state, syntax_set } = self;
        if line == 0 {
            let view_id = view.get_id();
            let text = view.get_line(0).unwrap_or("");
            let scope = view_state
                .get_mut(&view_id)
                .and_then(|state| state.compute_syntax(&text, None, syntax_set, false))
                .map(|(_, scope)| scope)?;
            Some(syntax_set.metadata().metadata_for_scope(scope.as_slice()))
        } else {
            let scope = view.get(line)?;
            scope
                .as_ref()
                .map(|(_, scope)| syntax_set.metadata().metadata_for_scope(scope.as_slice()))
        }
    }

    /// Checks for possible autoindent changes after an appropriate edit.
    fn consider_indentation(&mut self, view: &mut MyView, delta: &RopeDelta, edit_type: &str) {
        for region in delta.iter_inserts() {
            let line_of_edit = view.line_of_offset(region.new_offset).unwrap();
            let result = match edit_type {
                "newline" => self.autoindent_line(view, line_of_edit + 1),
                "insert" => {
                    let range = region.new_offset..region.new_offset + region.len;
                    let is_whitespace = {
                        let insert_region =
                            view.get_region(range).expect("view must return region");
                        insert_region.as_bytes().iter().all(u8::is_ascii_whitespace)
                    };
                    if !is_whitespace {
                        self.check_indent_active_edit(view, line_of_edit)
                    } else {
                        Ok(())
                    }
                }
                other => panic!("unexpected edit_type {}", other),
            };

            if let Err(e) = result {
                eprintln!("error in autoindent {:?}", e);
            }
        }
    }

    /// Called when freshly computing a line's indent level, such as after
    /// a newline, or when reindenting a block.
    fn autoindent_line(&mut self, view: &mut MyView, line: usize) -> Result<(), Error> {
        let _t = trace_block("Syntect::autoindent", &["syntect"]);
        debug_assert!(line > 0);
        let tab_size = view.get_config().tab_size;
        let current_indent = self.indent_level_of_line(view, line);
        let base_indent = self
            .previous_nonblank_line(view, line)?
            .map(|l| self.indent_level_of_line(view, l))
            .unwrap_or(0);
        let increase_level = self.test_increase(view, line)?;
        let indent_level = if increase_level { base_indent + tab_size } else { base_indent };
        if indent_level != current_indent {
            self.set_indent(view, line, indent_level)
        } else {
            Ok(())
        }
    }

    /// Called when actviely editing a line; cheifly checks for whether or not
    /// the current line should be de-indented, such as after a closeing '}'.
    fn check_indent_active_edit(&mut self, view: &mut MyView, line: usize) -> Result<(), Error> {
        let _t = trace_block("Syntect::check_indent_active_line", &["syntect"]);
        if line == 0 {
            return Ok(());
        }
        let tab_size = view.get_config().tab_size;
        let current_indent = self.indent_level_of_line(view, line);
        if line == 0 || current_indent == 0 {
            return Ok(());
        }
        let just_increased = self.test_increase(view, line)?;
        let decrease = self.test_decrease(view, line)?;
        let prev_line = self.previous_nonblank_line(view, line)?;
        let mut indent_level = prev_line.map(|l| self.indent_level_of_line(view, l)).unwrap_or(0);
        if decrease {
            // the first line after an increase should just match the previous line
            if !just_increased {
                indent_level = indent_level.saturating_sub(tab_size);
            }
            // we don't want to change indent level if this line doesn't
            // match `test_decrease`, because the user could have changed
            // it manually, and we respect that.
            if indent_level != current_indent {
                return self.set_indent(view, line, indent_level);
            }
        }
        Ok(())
    }

    fn set_indent(&self, view: &mut MyView, line: usize, level: usize) -> Result<(), Error> {
        let edit_start = view.offset_of_line(line)?;
        let edit_len = {
            let line = view.get_line(line)?;
            line.as_bytes().iter().take_while(|b| **b == b' ' || **b == b'\t').count()
        };

        let use_spaces = view.get_config().translate_tabs_to_spaces;
        let tab_size = view.get_config().tab_size;

        let indent_text = if use_spaces { n_spaces(level) } else { n_tabs(level / tab_size) };

        let iv = Interval::new(edit_start, edit_start + edit_len);
        let delta = RopeDelta::simple_edit(iv, indent_text.into(), view.get_buf_size());
        view.edit(delta, INDENTATION_PRIORITY, false, false, String::from("syntect"));
        Ok(())
    }

    /// Test whether the indent level should be increased for this line,
    /// by testing the _previous_ line against a regex.
    fn test_increase(&mut self, view: &mut MyView, line: usize) -> Result<bool, Error> {
        debug_assert!(line > 0, "increasing indent requires a previous line");
        let prev_line = match self.previous_nonblank_line(view, line) {
            Ok(Some(l)) => l,
            Ok(None) => return Ok(false),
            Err(e) => return Err(e),
        };
        let metadata = self.get_metadata(view, prev_line).ok_or_else(|| Error::PeerDisconnect)?;
        let line = view.get_line(prev_line)?;
        Ok(metadata.increase_indent(line))
    }

    /// Test whether the indent level for this line should be decreased, by
    /// checking this line against a regex.
    fn test_decrease(&mut self, view: &mut MyView, line: usize) -> Result<bool, Error> {
        if line == 0 {
            return Ok(false);
        }
        let metadata = self.get_metadata(view, line).ok_or_else(|| Error::PeerDisconnect)?;
        let line = view.get_line(line)?;
        Ok(metadata.decrease_indent(line))
    }

    fn previous_nonblank_line(
        &self,
        view: &mut MyView,
        line: usize,
    ) -> Result<Option<usize>, Error> {
        debug_assert!(line > 0);
        let mut line = line;
        while line > 0 {
            line -= 1;
            let text = view.get_line(line)?;
            if !text.bytes().all(|b| b.is_ascii_whitespace()) {
                return Ok(Some(line));
            }
        }
        Ok(None)
    }

    fn indent_level_of_line(&self, view: &mut MyView, line: usize) -> usize {
        let tab_size = view.get_config().tab_size;
        let line = view.get_line(line).unwrap_or("");
        line.as_bytes()
            .iter()
            .take_while(|b| **b == b' ' || **b == b'\t')
            .map(|b| if b == &b' ' { 1 } else { tab_size })
            .sum()
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
        edit_type: String,
        author: String,
    ) {
        let _t = trace_block("Syntect::update", &["syntect"]);
        view.schedule_idle();
        let should_auto_indent = view.get_config().auto_indent;
        if should_auto_indent
            && author == "core"
            && (edit_type == "newline" || edit_type == "insert")
        {
            if let Some(delta) = delta {
                self.consider_indentation(view, delta, &edit_type);
            }
        }
    }

    fn idle(&mut self, view: &mut View<Self::Cache>) {
        let state = self.view_state.get_mut(&view.get_id()).unwrap();
        for _ in 0..LINES_PER_RPC {
            if !state.highlight_one_line(view, self.syntax_set) {
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

fn n_spaces(n: usize) -> &'static str {
    // when someone opens an issue complaining about this we know we've made it
    const MAX_SPACES: usize = 160;
    static MANY_SPACES: [u8; MAX_SPACES] = [b' '; MAX_SPACES];
    unsafe { ::std::str::from_utf8_unchecked(&MANY_SPACES[..n.min(MAX_SPACES)]) }
}

fn n_tabs(n: usize) -> &'static str {
    const MAX_TABS: usize = 40;
    static MANY_TABS: [u8; MAX_TABS] = [b'\t'; MAX_TABS];
    unsafe { ::std::str::from_utf8_unchecked(&MANY_TABS[..n.min(MAX_TABS)]) }
}
