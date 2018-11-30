// Copyright 2018 The xi-editor Authors.
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

use parser::Parser;
use statestack::{Context, NewState, State};
use ScopeId;

const PLAINTEXT_SOURCE_SCOPE: &[&str] = &["source.plaintext"];

pub struct PlaintextParser<N> {
    scope_offset: Option<u32>,
    ctx: Context<(), N>,
}

impl<N: NewState<()>> PlaintextParser<N> {
    pub fn new(new_state: N) -> PlaintextParser<N> {
        PlaintextParser { scope_offset: None, ctx: Context::new(new_state) }
    }
}

impl<N: NewState<()>> Parser for PlaintextParser<N> {
    fn has_offset(&mut self) -> bool {
        self.scope_offset.is_some()
    }

    fn set_scope_offset(&mut self, offset: u32) {
        if !self.has_offset() {
            self.scope_offset = Some(offset)
        }
    }

    fn get_all_scopes(&self) -> Vec<Vec<String>> {
        vec![PLAINTEXT_SOURCE_SCOPE.iter().map(|it| it.to_string()).collect()]
    }

    fn get_scope_id_for_state(&self, _state: State) -> ScopeId {
        self.scope_offset.unwrap_or_default()
    }

    fn parse(&mut self, text: &str, state: State) -> (usize, State, usize, State) {
        (0, self.ctx.push(state, ()), text.as_bytes().len(), state)
    }
}
