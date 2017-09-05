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
#[macro_use]
extern crate xi_plugin_lib;

mod stackmap;

use xi_plugin_lib::state_cache::{self, PluginCtx};
use xi_plugin_lib::plugin_base::ScopeSpan;
use syntect::parsing::{ParseState, ScopeStack, SyntaxSet, SCOPE_REPO};
use stackmap::{StackMap, LookupResult};


struct PluginState<'a> {
    syntax_set: &'a SyntaxSet,
    stack_idents: StackMap,
    line_num: usize,
    offset: usize,
    initial_state: Option<ParseState>,
    parse_state: Option<ParseState>,
    scope_state: ScopeStack,
    spans_start: usize,
    // unflushed spans
    spans: Vec<ScopeSpan>,
    new_scopes: Vec<Vec<String>>,
    syntax_name: String,
}


impl<'a> PluginState<'a> {
    pub fn new(syntax_set: &'a SyntaxSet) -> Self {
        PluginState {
            syntax_set: syntax_set,
            stack_idents: StackMap::default(),
            line_num: 0,
            offset: 0,
            initial_state: None,
            parse_state: None,
            scope_state: ScopeStack::new(),
            spans_start: 0,
            spans: Vec::new(),
            new_scopes: Vec::new(),
            syntax_name: String::from("None"),
        }
    }

    // compute syntax for one line, also accumulating the style spans
    fn compute_syntax(&mut self, line: &str, state: State) -> State {
        let mut parse_state = state.or_else(|| self.initial_state.clone()).unwrap();
        let ops = parse_state.parse_line(&line);

        let mut prev_cursor = 0;
        let repo = SCOPE_REPO.lock().unwrap();
        for (cursor, batch) in ops {
            if self.scope_state.len() > 0 {
                let scope_ident = self.stack_idents.get_value(self.scope_state.as_slice());
                let scope_ident = match scope_ident {
                    LookupResult::Existing(id) => id,
                    LookupResult::New(id) => {
                        let stack_strings = self.scope_state.as_slice().iter()
                            .map(|slice| repo.to_string(*slice))
                            .collect::<Vec<_>>();
                        self.new_scopes.push(stack_strings);
                        id
                    }
                };

                let start = self.offset - self.spans_start + prev_cursor;
                let end = start + (cursor - prev_cursor);
                let span = ScopeSpan::new(start, end, scope_ident);
                self.spans.push(span);
            }
            prev_cursor = cursor;
            self.scope_state.apply(&batch);
        }
        Some(parse_state)
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
                    Some((new_state, line_num + 1))
                }
                Err(_) => None,
            };
            if let Some((new_state, new_frontier)) = new_frontier {
                ctx.set(new_frontier, new_state);
                ctx.update_frontier(new_frontier);
                return true;
            } else {
                ctx.close_frontier();
            }
        }
        false
    }

    fn flush_spans(&mut self, ctx: &mut PluginCtx<State>) {
        if !self.new_scopes.is_empty() {
            ctx.add_scopes(&self.new_scopes);
            self.new_scopes.clear();
        }
        if !self.spans.is_empty() {
            ctx.update_spans(self.spans_start, self.offset - self.spans_start,
                             self.spans.as_slice());
            self.spans.clear();
        }
        self.spans_start = self.offset;
    }

    fn do_highlighting(&mut self, mut ctx: PluginCtx<State>) {
        let syntax = match ctx.get_path() {
            Some(ref path) => self.syntax_set.find_syntax_for_file(path).unwrap()
                .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text()),
            None => self.syntax_set.find_syntax_plain_text(),
        };

        if syntax.name != self.syntax_name {
            self.syntax_name = syntax.name.clone();
            print_err!("syntect using {}", syntax.name);
        }

        self.initial_state = Some(ParseState::new(syntax));
        self.parse_state = self.initial_state.clone();
        self.scope_state = ScopeStack::new();
        self.spans = Vec::new();
        self.new_scopes = Vec::new();
        self.line_num = 0;
        self.offset = 0;
        self.spans_start = 0;
        ctx.schedule_idle(0);
    }
}

const LINES_PER_RPC: usize = 50;

// TODO: this needs to be option because the caching layer relies on Default.
// We can't implement that because the actual initial state depends on the
// syntax. There are other ways to handle this, but this will do for now.
type State = Option<ParseState>;

impl<'a> state_cache::Handler for PluginState<'a> {
    type State = State;

    fn initialize(&mut self, ctx: PluginCtx<State>, _buf_size: usize) {
        self.do_highlighting(ctx);
    }

    fn update(&mut self, ctx: PluginCtx<State>) {
        self.do_highlighting(ctx);
    }

    fn did_save(&mut self, ctx: PluginCtx<State>) {
        self.do_highlighting(ctx);
    }

    fn idle(&mut self, mut ctx: PluginCtx<State>, _token: usize) {
        //print_err!("idle task at line {}", self.line_num);
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
    let syntax_set = SyntaxSet::load_defaults_newlines();
    let mut state = PluginState::new(&syntax_set);

    state_cache::mainloop(&mut state);
}
