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

extern crate serde;
extern crate syntect;
#[macro_use]
extern crate xi_plugin_lib;

mod stackmap;

use xi_plugin_lib::caching_plugin::{self, PluginCtx};
use xi_plugin_lib::plugin_base::ScopeSpan;
use syntect::parsing::{ParseState, ScopeStack, SyntaxSet};
use stackmap::{StackMap, LookupResult};


struct PluginState<'a> {
    syntax_set: &'a SyntaxSet,
    stack_idents: StackMap,
    line_num: usize,
    offset: usize,
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
            parse_state: None,
            scope_state: ScopeStack::new(),
            spans_start: 0,
            spans: Vec::new(),
            new_scopes: Vec::new(),
            syntax_name: String::from("None"),
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
        if self.spans.is_empty() {
            self.spans_start = self.offset;
        }

        //print_err!("\n\nline {}\napplying ops {:?}", &line, &ops);
        let mut prev_cursor = 0;
        for (cursor, batch) in ops {
            if self.scope_state.len() > 0 {
                let scope_ident = self.stack_idents.get_value(self.scope_state.as_slice());
                //print_err!("scope ident: {:?}", scope_ident);
                let scope_ident = match scope_ident {
                    LookupResult::Existing(id) => id,
                    LookupResult::New(id) => {
                        let stack_strings = self.scope_state.as_slice().iter()
                            .map(|slice| slice.build_string())
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

        self.line_num += 1;
        self.offset += line.len();
        true
    }

    fn flush_spans(&mut self, ctx: &mut PluginCtx) {
        if !self.new_scopes.is_empty() {
            ctx.add_scopes(&self.new_scopes);
        }
        if !self.spans.is_empty() {
            ctx.update_spans(self.spans_start, self.offset - self.spans_start,
                             self.spans.as_slice());
        }
    }

    fn do_highlighting(&mut self, mut ctx: PluginCtx) {
        let syntax = match ctx.get_path() {
            Some(ref path) => self.syntax_set.find_syntax_for_file(path).unwrap()
                .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text()),
            None => self.syntax_set.find_syntax_plain_text(),
        };

        if syntax.name != self.syntax_name {
            self.syntax_name = syntax.name.clone();
            print_err!("syntect using {}", syntax.name);
        }

        self.parse_state = Some(ParseState::new(syntax));
        self.scope_state = ScopeStack::new();
        self.spans = Vec::new();
        self.new_scopes = Vec::new();
        self.line_num = 0;
        self.offset = 0;
        ctx.schedule_idle(0);
    }
}

const LINES_PER_RPC: usize = 50;

impl<'a> caching_plugin::Handler for PluginState<'a> {
    fn initialize(&mut self, ctx: PluginCtx, _buf_size: usize) {
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
    let syntax_set = SyntaxSet::load_defaults_newlines();
    let mut state = PluginState::new(&syntax_set);

    caching_plugin::mainloop(&mut state);
}
