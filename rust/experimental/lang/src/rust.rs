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

//! Rust language syntax analysis and highlighting.

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
    Keyword,
    Operator,
    PrimType,
    //RawStrHash,  // One for each hash in a raw string
    //Block,    // One for each {
    //Bracket,  // One for each [
    //Paren,    // One for each (
    // generics etc
}

pub fn to_style(style: &mut Style, el: &StateEl) {
    match *el {
        StateEl::Comment => style.fg_color = 0xFF75715E,
        StateEl::StrQuote => style.fg_color = 0xFF998844,
        StateEl::CharQuote => style.fg_color = 0xFF998844,
        StateEl::Invalid => style.fg_color = 0xFFFF0000,
        StateEl::NumericLiteral => style.fg_color = 0xFF6644EE,
        StateEl::CharConst => style.fg_color = 0xFF8866EE,
        StateEl::Keyword => style.font = 1,
        StateEl::Operator => style.fg_color = 0xFFAA2244,
        StateEl::PrimType => style.fg_color = 0xFF44AAAA,
    }
}

// sorted for easy binary searching
const RUST_KEYWORDS: &'static [&'static [u8]] = &[
    b"Self", b"abstract", b"alignof", b"as", b"become", b"box", b"break",
    b"const", b"continue", b"crate", b"default", b"do", b"else", b"enum",
    b"extern", b"false", b"final", b"fn", b"for", b"if", b"impl", b"in", b"let",
    b"loop", b"macro", b"match", b"mod", b"move", b"mut", b"offsetof",
    b"override", b"priv", b"proc", b"pub", b"pure", b"ref", b"return", b"self",
    b"sizeof", b"static", b"struct", b"super", b"trait", b"true", b"type",
    b"typeof", b"union", b"unsafe", b"unsized", b"use", b"virtual", b"where",
    b"while", b"yield"
];

// sorted for easy binary searching
const RUST_PRIM_TYPES: &'static [&'static [u8]] = &[
    b"bool", b"char", b"f32", b"f64", b"i128", b"i16", b"i32", b"i64", b"i8",
    b"isize", b"str", b"u128", b"u16", b"u32", b"u64", b"u8", b"usize"
];

const RUST_OPERATORS: &'static [&'static [u8]] = &[
    b"!", b"%=", b"%", b"&=", b"&&", b"&", b"*=", b"*", b"+=", b"+", b"-=", b"-",
    b"/=", b"/", b"<<=", b"<<", b">>=", b">>", b"^=", b"^", b"|=", b"||", b"|",
    b"==", b"=", b"..", b"=>", b"<=", b"<", b">=", b">"
];

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

    fn quoted_str(&mut self, t: &[u8], state: State) -> (usize, State, usize, State) {
        let mut i = 0;
        while i < t.len() {
            let b = t[i];
            if b == b'"' {
                return (0, state, i + 1, self.ctx.pop(state).unwrap());
            } else if b == b'\\' {
                if let Some(len) = escape.p(&t[i..]) {
                    return (i, self.ctx.push(state, StateEl::CharConst), len, state);
                } else if let Some(len) = 
                        (FailIf(OneOf(b"\r\nbu")), OneChar(|_| true)).p(&t[i+1..]) {
                    return (i + 1, self.ctx.push(state, StateEl::Invalid), len, state);
                }
            }
            i += 1;
        }
        (0, state, i, state)
    }
}

fn is_digit(c: u8) -> bool {
    c >= b'0' && c <= b'9'
}

fn is_hex_digit(c: u8) -> bool {
    (c >= b'0' && c <= b'9') || (c >= b'a' && c <= b'f') || (c >= b'A' && c <= b'F')
}

// Note: will have to rework this if we want to support non-ASCII identifiers
fn is_ident_start(c: u8) -> bool {
    (c >= b'A' && c <= b'Z') || (c >= b'a' && c <= b'z') || c == b'_'
}

fn is_ident_continue(c: u8) -> bool {
    is_ident_start(c) || is_digit(c)
}

fn ident(s: &[u8]) -> Option<usize> {
    (OneByte(is_ident_start), ZeroOrMore(OneByte(is_ident_continue))).p(s)
}

// sequence of decimal digits with optional separator
fn raw_numeric(s: &[u8]) -> Option<usize> {
    (OneByte(is_digit), ZeroOrMore(Alt(b'_', OneByte(is_digit)))).p(s)
}

fn int_suffix(s: &[u8]) -> Option<usize> {
    (Alt(b'u', b'i'), OneOf(&["8", "16", "32", "64", "128", "size"])).p(s)
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
        b'0',
        Alt3(
            (b'x', OneOrMoreWithSep(OneByte(is_hex_digit), b'_')),
            (b'o', OneOrMoreWithSep(Inclusive(b'0'..b'7'), b'_')),
            (b'b', OneOrMoreWithSep(Alt(b'0', b'1'), b'_')),
        ),
        Optional(int_suffix)
    ).p(s)
}

fn positive_decimal(s: &[u8]) -> Option<usize> {
    (
        raw_numeric,
        Alt(int_suffix,
            (
                Optional((b'.', FailIf(OneByte(is_ident_start)), Optional(raw_numeric))),
                Optional((Alt(b'e', b'E'), Optional(Alt(b'+', b'-')), raw_numeric)),
                Optional(Alt("f32", "f64"))
            )
        )
    ).p(s)
}

fn numeric_literal(s: &[u8]) -> Option<usize> {
    (Optional(b'-'), Alt(positive_nondecimal, positive_decimal)).p(s)
}

fn escape(s: &[u8]) -> Option<usize> {
    (
        b'\\',
        Alt3(
            OneOf(b"\\\'\"0nrt"),
            (b'x', Repeat(OneByte(is_hex_digit), 2)),
            ("u{", Repeat(OneByte(is_hex_digit), 1..7), b'}')
        )
    ).p(s)
}

fn char_literal(s: &[u8]) -> Option<usize> {
    (
        b'\'',
        Alt(OneChar(|c| c != '\\' && c != '\''), escape),
        b'\''
    ).p(s)
}

impl<N: NewState<StateEl>> Colorize for RustColorize<N> {
    fn colorize(&mut self, text: &str, mut state: State) -> (usize, State, usize, State) {
        let t = text.as_bytes();
        match self.ctx.tos(state) {
            Some(StateEl::Comment) => {
                for i in 0..t.len() {
                    if let Some(len) = "/*".p(&t[i..]) {
                        state = self.ctx.push(state, StateEl::Comment);
                        return (i, state, len, state);
                    } else if let Some(len) = "*/".p(&t[i..]) {
                        return (0, state, i + len, self.ctx.pop(state).unwrap());
                    }
                }
                return (0, state, t.len(), state);
            }
            Some(StateEl::StrQuote) => return self.quoted_str(t, state),
            _ => ()
        }
        let mut i = 0;
        while i < t.len() {
            let b = t[i];
            if let Some(len) = "/*".p(&t[i..]) {
                state = self.ctx.push(state, StateEl::Comment);
                return (i, state, len, state);
            } else if let Some(_) = "//".p(&t[i..]) {
                return (i, self.ctx.push(state, StateEl::Comment), t.len(), state);
            } else if let Some(len) = numeric_literal.p(&t[i..]) {
                return (i, self.ctx.push(state, StateEl::NumericLiteral), len, state);
            } else if b == b'"' {
                state = self.ctx.push(state, StateEl::StrQuote);
                return (i, state, 1, state);
            } else if let Some(len) = char_literal.p(&t[i..]) {
                return (i, self.ctx.push(state, StateEl::CharQuote), len, state);
            } else if let Some(len) = OneOf(RUST_OPERATORS).p(&t[i..]) {
                return (i, self.ctx.push(state, StateEl::Operator), len, state);
            } else if let Some(len) = ident.p(&t[i..]) {
                if RUST_KEYWORDS.binary_search(&&t[i..i + len]).is_ok() {
                    return (i, self.ctx.push(state, StateEl::Keyword), len, state);
                } else if RUST_PRIM_TYPES.binary_search(&&t[i..i + len]).is_ok() {
                    return (i, self.ctx.push(state, StateEl::PrimType), len, state);
                } else {
                    i += len;
                    continue;
                }
            }
            i += 1;
        }
        (0, state, t.len(), state)
    }
}

// A simple stdio based harness for testing.
pub fn test() {
    let mut buf = String::new();
    let _ = stdin().read_to_string(&mut buf).unwrap();
    let mut c = RustColorize::new(DebugNewState::new());
    colorize::run_debug(&mut c, &buf);
}

#[cfg(test)]
mod tests {
    use super::numeric_literal;

    #[test]
    fn numeric_literals() {
        assert_eq!(Some(1), numeric_literal(b"2.f64"));
        assert_eq!(Some(6), numeric_literal(b"2.0f64"));
        assert_eq!(Some(1), numeric_literal(b"2._f64"));
        assert_eq!(Some(1), numeric_literal(b"2._0f64"));
        assert_eq!(Some(5), numeric_literal(b"1_2__"));
        assert_eq!(Some(7), numeric_literal(b"1_2__u8"));
        assert_eq!(Some(9), numeric_literal(b"1_2__u128"));
        assert_eq!(None, numeric_literal(b"_1_"));
        assert_eq!(Some(4), numeric_literal(b"0xff"));
        assert_eq!(Some(4), numeric_literal(b"0o6789"));
    }
}
