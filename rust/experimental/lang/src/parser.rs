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

use crate::statestack::State;
use crate::ScopeId;

/// Trait for abstracting over text parsing and [Scope] extraction
pub trait Parser {
    fn has_offset(&mut self) -> bool;
    fn set_scope_offset(&mut self, offset: u32);
    fn get_all_scopes(&self) -> Vec<Vec<String>>;
    fn get_scope_id_for_state(&self, state: State) -> ScopeId;
    fn parse(&mut self, text: &str, state: State) -> (usize, State, usize, State);
}
