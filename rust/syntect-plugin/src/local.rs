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

use std::sync::MutexGuard;
use std::borrow::Cow;

use serde_json::Value;

use xi_plugin_lib::state_cache::{self, PluginCtx};
use xi_core_lib::plugin_rpc::ScopeSpan;
use xi_rope::rope::RopeDelta;
use xi_rope::interval::Interval;
use xi_rope::delta::Builder as EditBuilder;

use syntect::parsing::{ParseState, ScopeStack, SyntaxSet, SCOPE_REPO, ScopeRepository};
use stackmap::{StackMap, LookupResult};


/// The state for syntax highlighting of one file.
struct PluginState<'a> {
    syntax_set: &'a SyntaxSet,
    stack_idents: StackMap,
    offset: usize,
    initial_state: Option<(ParseState, ScopeStack)>,
    spans_start: usize,
    // unflushed spans
    spans: Vec<ScopeSpan>,
    new_scopes: Vec<Vec<String>>,
    syntax_name: String,
}

const LINES_PER_RPC: usize = 10;
const INDENTATION_PRIORITY: usize = 100;

type LockedRepo = MutexGuard<'static, ScopeRepository>;

/// The syntax highlighting state corresponding to the beginning of a line
/// (as stored in the state cache).
// Note: this needs to be option because the caching layer relies on Default.
// We can't implement that because the actual initial state depends on the
// syntax. There are other ways to handle this, but this will do for now.
type State = Option<(ParseState, ScopeStack)>;


impl<'a> PluginState<'a> {
    pub fn new(syntax_set: &'a SyntaxSet) -> Self {
        PluginState {
            syntax_set: syntax_set,
            stack_idents: StackMap::default(),
            offset: 0,
            initial_state: None,
            spans_start: 0,
            spans: Vec::new(),
            new_scopes: Vec::new(),
            syntax_name: String::from("None"),
        }
    }

    // compute syntax for one line, also accumulating the style spans
    fn compute_syntax(&mut self, line: &str, state: State) -> State {
        let (mut parse_state, mut scope_state) = state.or_else(|| self.initial_state.clone()).unwrap();
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
                let stack_strings = stack.as_slice().iter()
                    .map(|slice| repo.to_string(*slice))
                    .collect::<Vec<_>>();
                self.new_scopes.push(stack_strings);
                id
            }
        }
    }

    #[allow(unused)]
    // Return true if there's any more work to be done.
    fn highlight_one_line(&mut self, ctx: &mut PluginCtx<State>) -> bool {
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

    fn flush_spans(&mut self, ctx: &mut PluginCtx<State>) {
        if !self.new_scopes.is_empty() {
            ctx.add_scopes(&self.new_scopes);
            self.new_scopes.clear();
        }
        if self.spans_start != self.offset {
            ctx.update_spans(self.spans_start, self.offset - self.spans_start,
                             &self.spans);
            self.spans.clear();
        }
        self.spans_start = self.offset;
    }

    fn do_highlighting(&mut self, mut ctx: PluginCtx<State>) {
        let syntax = match ctx.get_view().path {
            Some(ref path) => self.syntax_set.find_syntax_for_file(path).unwrap()
                .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text()),
            None => self.syntax_set.find_syntax_plain_text(),
        };

        if syntax.name != self.syntax_name {
            self.syntax_name = syntax.name.clone();
            eprintln!("syntect using {}", syntax.name);
        }

        self.initial_state = Some((ParseState::new(syntax), ScopeStack::new()));
        self.spans = Vec::new();
        self.new_scopes = Vec::new();
        self.offset = 0;
        self.spans_start = 0;
        ctx.reset();
        ctx.schedule_idle(0);
    }

    /// Checks if a newline has been inserted, and if so inserts whitespace
    /// as necessary.
    fn do_indentation(&mut self, ctx: &mut PluginCtx<State>, start: usize,
                      end: usize, rev: usize, text: &str) -> Option<Value> {
        // don't touch indentation if this is not a simple edit
        if end != start { return None }

        let line_ending = ctx.get_config().line_ending.clone();
        let is_newline = line_ending == text;

        if is_newline {
            let line_num = ctx.find_offset(start).err();

            let use_spaces = ctx.get_config().translate_tabs_to_spaces;
            let tab_size = ctx.get_config().tab_size;
            let buf_size = ctx.get_buf_size();
            if let Some(line) = line_num.and_then(|idx| ctx.get_line(idx).ok()) {
                // do not send update if last line is empty string (contains only line ending)
                if line == line_ending { return None }

                let indent = self.indent_for_next_line(
                    line, use_spaces, tab_size);
                let ix = start + text.len();
                let interval = Interval::new_open_closed(ix, ix);
                let mut builder = EditBuilder::new(buf_size);
                builder.replace(interval, indent.into());
                let delta = builder.build();
                let edit = json!({
                    "rev": rev,
                    "delta": delta,
                    "priority": INDENTATION_PRIORITY,
                    "after_cursor": false,
                    "author": "syntect",
                });
                return Some(edit)
            }
        }
        None
    }

    /// Returns the string which should be inserted after the newline
    /// to achieve the desired indentation level.
    fn indent_for_next_line<'b>(&self, prev_line: &'b str, use_spaces: bool,
                                tab_size: usize) -> Cow<'b, str> {
        let leading_ws = prev_line.char_indices()
            .find(|&(_, c)| !c.is_whitespace())
            .or(prev_line.char_indices().last())
            .map(|(idx, _)| unsafe { prev_line.slice_unchecked(0, idx) } )
            .unwrap_or("");

        if self.increase_indentation(prev_line) {
            let indent_text = if use_spaces {
                &"                                    "[..tab_size]
            } else {
                "\t"
            };
            format!("{}{}", leading_ws, indent_text).into()
        } else {
            leading_ws.into()
        }
    }

    /// Checks if the indent level should be increased.
    fn increase_indentation(&self, prev_line: &str) -> bool {
        let trailing_char = prev_line.trim_right().chars()
            .rev().next().unwrap_or(' ');
        // very naive heuristic for modifying indentation level.
        match trailing_char {
            '{' | ':' => true,
            _ => false,
        }
    }
}

impl<'a> state_cache::Plugin for PluginState<'a> {
    type State = State;

    fn initialize(&mut self, ctx: PluginCtx<State>, _buf_size: usize) {
        self.do_highlighting(ctx);
    }

    fn update(&mut self, mut ctx: PluginCtx<State>, rev: usize,
              delta: Option<RopeDelta>) -> Option<Value> {
        ctx.schedule_idle(0);
        let should_auto_indent = ctx.get_config().auto_indent;
        if should_auto_indent {
            if let Some(delta) = delta {
                let (iv, _) = delta.summary();
                if let Some(s) = delta.as_simple_insert() {
                    let s: String = s.into();
                    return self.do_indentation(&mut ctx, iv.start(), iv.end(), rev, &s)
                }
            }
        }
        None
    }

    fn did_save(&mut self, ctx: PluginCtx<State>) {
        // TODO: use smarter logic to figure out whether we need to re-highlight the whole file
        self.do_highlighting(ctx);
    }

    fn idle(&mut self, mut ctx: PluginCtx<State>, _token: usize) {
        //eprintln!("idle task at offset {}", self.offset);
        for _ in 0..LINES_PER_RPC {
            if !self.highlight_one_line(&mut ctx) {
                self.flush_spans(&mut ctx);
                return;
            }
            if ctx.request_is_pending() {
                eprintln!("request pending at offset {}", self.offset);
                break;
            }
        }
        self.flush_spans(&mut ctx);
        ctx.schedule_idle(0);
    }
}

#[allow(dead_code)]
pub(crate) fn main() {
    let syntax_set = SyntaxSet::load_defaults_newlines();
    let mut state = PluginState::new(&syntax_set);

    let _ = state_cache::mainloop(&mut state);
}
