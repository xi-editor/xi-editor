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

//! The trait for explicit state syntax coloring, and some support.

use std::fmt::Debug;

use statestack::{State, NewState};

pub trait Colorize {
    /// first state is for the highlighted span; size of highlighted span; next state
    /// invariants: text is not empty; text contains at most one line
    /// (TODO: send empty string to represent EOF?)
    ///
    /// Return value:
    ///   number of bytes matched at previous state
    ///   state of this match
    ///   number of bytes in this match
    ///   next state
    fn colorize(&mut self, text: &str, state: State) -> (usize, State, usize, State);
}

pub fn run_debug<C: Colorize>(c: &mut C, s: &str) {
    let mut state = State::default();
    for line in s.lines() {
        let mut i = 0;
        while i < line.len() {
            let (prevlen, s0, len, s1) = c.colorize(&line[i..], state);
            if prevlen > 0 {
               println!("{}: {:?}", &line[i..i + prevlen], state);
               i += prevlen;
            }
            println!("{}: {:?}", &line[i..i + len], s0);
            i += len;
            state = s1;
        }
    }
}

pub struct DebugNewState;

impl DebugNewState {
    pub fn new() -> DebugNewState {
        DebugNewState
    }
}

impl<T: Debug> NewState<T> for DebugNewState {
    fn new_state(&mut self, s: State, contents: &[T]) {
        println!("new state {:?}: {:?}", s, contents);
    }
}

#[derive(Clone, Debug)]
pub struct Style {
    pub fg_color: u32, // ARGB
    pub font: u8, // bitflags, 1=bold, 2=underline, 4=italic
}

impl Default for Style {
    fn default() -> Style {
        Style {
            fg_color: 0xff000000,
            font: 0,
        }
    }
}

pub struct StyleNewState<F> {
    to_style: F,
    styles: Vec<Style>
}

impl<F> StyleNewState<F> {
    pub fn new(to_style: F) -> StyleNewState<F> {
        StyleNewState {
            to_style: to_style,
            styles: vec![Style::default()]
        }
    }

    pub fn get_style(&self, s: State) -> &Style {
        &self.styles[s.raw()]
    }
}

impl<T, F: Fn(&mut Style, &T)> NewState<T> for StyleNewState<F> {
    fn new_state(&mut self, _s: State, contents: &[T]) {
        let mut style = Style::default();
        for el in contents {
            (self.to_style)(&mut style, el);
        }
        self.styles.push(style);
    }
}
