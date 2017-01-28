// Copyright 2017 Google Inc. All rights reserved.
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

use std::io::{stdin, Read};

use statestack::{State, Context, NewState};
use colorize::{self, Colorize, DebugNewState, Style};
use peg::*;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum StateEl {
    StrQuote,
    CharQuote,
    Comment,  // One for each /*
    CharConst,
    NumericLiteral,
    Invalid,
    //RawStrHash,  // One for each hash in a raw string
    //Block,    // One for each {
    //Bracket,  // One for each [
    //Paren,    // One for each (
    // generics etc
}

pub fn to_style(style: &mut Style, el: &StateEl) {
    match *el {
        StateEl::Comment => style.fg_color = 0xFF75715E,
        StateEl::StrQuote => style.fg_color = 0xFFE6DB74,
        StateEl::CharQuote => style.fg_color = 0xFFE6DB74,
        StateEl::Invalid => style.fg_color = 0xFFFF0000,
        StateEl::NumericLiteral => style.fg_color = 0xFFAE81FF,
        StateEl::CharConst => style.fg_color = 0xFFAE81FF,
    }
}

pub struct RustColorize<N> {
    ctx: Context<StateEl, N>,
}

impl<N: NewState<StateEl>> RustColorize<N> {
    pub fn new(new_state: N) -> RustColorize<N> {
        RustColorize {
            ctx: Context::new(new_state),
        }
    }

    pub fn get_new_state(&self) -> &N {
        self.ctx.get_new_state()
    }

    fn quoted_str(&mut self, t: &[u8], state: State) -> (State, usize, State) {
        let mut i = 0;
        while i < t.len() {
            let b = t[i];
            if b == b'"' {
                return (state, i + 1, self.ctx.pop(state).unwrap());
            } else if b == b'\\' {
                if let Some(len) = escape.p(&t[i..]) {
                    if i > 0 {
                        return (state, i, state);
                    }
                    return (self.ctx.push(state, StateEl::CharConst), len, state);
                } else if !is_eol(&t[i + 1 ..]) {
                    if i > 0 {
                        return (state, i, state);
                    }
                    return (self.ctx.push(state, StateEl::Invalid), 1, state);
                }
            }
            i += 1;
        }
        (state, i, state)
    }
}

fn is_digit(c: u8) -> bool {
    c >= b'0' && c <= b'9'
}

fn is_octal_digit(c: u8) -> bool {
    c >= b'0' && c <= b'7'
}

fn is_hex_digit(c: u8) -> bool {
    (c >= b'0' && c <= b'9') || (c >= b'a' && c <= b'f') || (c >= b'A' && c <= b'F')
}

// sequence of decimal digits with optional separator
fn raw_numeric(s: &[u8]) -> Option<usize> {
    (OneByte(is_digit), ZeroOrMore(Alt('_', OneByte(is_digit)))).p(s)
}

fn int_suffix(s: &[u8]) -> Option<usize> {
    (Alt('u', 'i'), OneOf(&["8", "16", "32", "64", "128", "size"])).p(s)
}

// At least one P with any number of SEP mixed in. Note: this is also an example
// of composing combinators to make a new combinator.
struct OneOrMoreWithSep<P, SEP>(P, SEP);

impl<P: Peg, SEP: Peg> Peg for OneOrMoreWithSep<P, SEP> {
    fn p(&self, s: &[u8]) -> Option<usize> {
        let OneOrMoreWithSep(ref p, ref sep) = *self;
        (ZeroOrMore(Ref(sep)), Ref(p), ZeroOrMore(Alt(Ref(p), Ref(sep)))).p(s)
    }
}

fn positive_nondecimal(s: &[u8]) -> Option<usize> {
    (
        '0',
        Alt3(
            ('x', OneOrMoreWithSep(OneByte(is_hex_digit), '_')),
            ('o', OneOrMoreWithSep(OneByte(is_octal_digit), '_')),
            ('b', OneOrMoreWithSep(Alt('0', '1'), '_')),
        ),
        Optional(int_suffix)
    ).p(s)
}

fn positive_decimal(s: &[u8]) -> Option<usize> {
    (
        raw_numeric,
        Alt(int_suffix,
            (
                Optional(('.', Optional(raw_numeric))),
                Optional((Alt('e', 'E'), Optional(Alt('+', '-')), raw_numeric)),
                Optional(Alt("f32", "f64"))
            )
        )
    ).p(s)
}

fn numeric_literal(s: &[u8]) -> Option<usize> {
    (Optional('-'), Alt(positive_nondecimal, positive_decimal)).p(s)
}

fn escape(s: &[u8]) -> Option<usize> {
    (
        '\\',
        Alt3(
            OneOf(b"\\\'\"0nrt"),
            ("x", Repeat(OneByte(is_hex_digit), 2, 2)),
            ("u{", Repeat(OneByte(is_hex_digit), 1, 6), "}")
        )
    ).p(s)
}

fn char_literal(s: &[u8]) -> Option<usize> {
    (
        '\'',
        Alt(OneChar(|c| c != '\\' && c != '\''), escape),
        '\''
    ).p(s)
}

fn is_eol(s: &[u8]) -> bool {
    if s.is_empty() {
        true
    } else {
        let b = s[0];
        b == b'\r' || b == b'\n'
    }
}

impl<N: NewState<StateEl>> Colorize for RustColorize<N> {
    fn colorize(&mut self, text: &str, mut state: State) -> (State, usize, State) {
        let t = text.as_bytes();
        match self.ctx.tos(state) {
            Some(StateEl::Comment) => {
                for i in 0..t.len() {
                    if let Some(len) = "/*".p(&t[i..]) {
                        if i > 0 {
                            return (state, i, state);
                        }
                        state = self.ctx.push(state, StateEl::Comment);
                        return (state, len, state);
                    } else if let Some(len) = "*/".p(&t[i..]) {
                        return (state, i + len, self.ctx.pop(state).unwrap());
                    }
                }
                return (state, t.len(), state);
            }
            Some(StateEl::StrQuote) => return self.quoted_str(t, state),
            _ => ()
        }
        for (i, &b) in t.iter().enumerate() {
            if let Some(len) = "/*".p(&t[i..]) {
                if i > 0 {
                    return (state, i, state);
                }
                state = self.ctx.push(state, StateEl::Comment);
                return (state, len, state);
            } else if let Some(_) = "//".p(&t[i..]) {
                if i > 0 {
                    return (state, i, state);
                }
                return (self.ctx.push(state, StateEl::Comment), t.len(), state)
            } else if let Some(len) = numeric_literal.p(&t[i..]) {
                if i > 0 {
                    return (state, i, state);
                }
                return (self.ctx.push(state, StateEl::NumericLiteral), len, state)
            } else if b == b'"' {
                if i > 0 {
                    return (state, i, state);
                }
                state = self.ctx.push(state, StateEl::StrQuote);
                return (state, 1, state)
            } else if let Some(len) = char_literal.p(&t[i..]) {
                if i > 0 {
                    return (state, i, state);
                }
                return (self.ctx.push(state, StateEl::CharQuote), len, state)
            }
        }
        (state, t.len(), state)
    }
}

// A simple stdio based harness for testing.
pub fn test() {
    let mut buf = String::new();
    let _ = stdin().read_to_string(&mut buf).unwrap();
    let mut c = RustColorize::new(DebugNewState::new());
    colorize::run_debug(&mut c, &buf);
}
