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

use statestack::State;
use Scope;

/// Trait for abstracting over text parsing and [Scope] extraction
pub trait Parser {
    fn get_all_scopes(&self) -> Vec<Scope>;
    fn get_scope_for_state(&self, state: State) -> u32;
    fn parse(&mut self, text: &str, state: State) -> (usize, State, usize, State);
}
