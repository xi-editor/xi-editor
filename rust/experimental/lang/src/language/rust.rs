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

use crate::parser::Parser;
use crate::peg::*;
use crate::statestack::{Context, State};
use crate::ScopeId;

/// See [this](https://github.com/sublimehq/Packages/blob/master/Rust/Rust.sublime-syntax)
/// for reference.
static ALL_SCOPES: &[&[&str]] = &[
    &["source.rust"],
    &["source.rust", "string.quoted.double.rust"],
    &["source.rust", "string.quoted.single.rust"],
    &["source.rust", "comment.line.double-slash.rust"],
    &["source.rust", "constant.character.escape.rust"],
    &["source.rust", "constant.numeric.decimal.rust"],
    &["source.rust", "invalid.illegal.rust"],
    &["source.rust", "keyword.operator.rust"],
    &["source.rust", "keyword.operator.arithmetic.rust"],
    &["source.rust", "entity.name.type.rust"],
];

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum StateEl {
    Source,
    StrQuote,
    CharQuote,
    Comment,
    // One for each /*
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

impl StateEl {
    pub fn scope_id(&self) -> ScopeId {
        match self {
            StateEl::Source => 0,
            StateEl::StrQuote => 1,
            StateEl::CharQuote => 2,
            StateEl::Comment => 3,
            StateEl::CharConst => 4,
            StateEl::NumericLiteral => 5,
            StateEl::Invalid => 6,
            StateEl::Keyword => 7,
            StateEl::Operator => 8,
            StateEl::PrimType => 9,
        }
    }
}

// sorted for easy binary searching
const RUST_KEYWORDS: &[&[u8]] = &[
    b"Self",
    b"abstract",
    b"alignof",
    b"as",
    b"become",
    b"box",
    b"break",
    b"const",
    b"continue",
    b"crate",
    b"default",
    b"do",
    b"else",
    b"enum",
    b"extern",
    b"false",
    b"final",
    b"fn",
    b"for",
    b"if",
    b"impl",
    b"in",
    b"let",
    b"loop",
    b"macro",
    b"match",
    b"mod",
    b"move",
    b"mut",
    b"offsetof",
    b"override",
    b"priv",
    b"proc",
    b"pub",
    b"pure",
    b"ref",
    b"return",
    b"self",
    b"sizeof",
    b"static",
    b"struct",
    b"super",
    b"trait",
    b"true",
    b"type",
    b"typeof",
    b"union",
    b"unsafe",
    b"unsized",
    b"use",
    b"virtual",
    b"where",
    b"while",
    b"yield",
];

// sorted for easy binary searching
const RUST_PRIM_TYPES: &[&[u8]] = &[
    b"bool", b"char", b"f32", b"f64", b"i128", b"i16", b"i32", b"i64", b"i8", b"isize", b"str",
    b"u128", b"u16", b"u32", b"u64", b"u8", b"usize",
];

const RUST_OPERATORS: &[&[u8]] = &[
    b"!", b"%=", b"%", b"&=", b"&&", b"&", b"*=", b"*", b"+=", b"+", b"-=", b"-", b"/=", b"/",
    b"<<=", b"<<", b">>=", b">>", b"^=", b"^", b"|=", b"||", b"|", b"==", b"=", b"..", b"=>",
    b"<=", b"<", b">=", b">",
];

pub struct RustParser {
    scope_offset: Option<u32>,
    ctx: Context<StateEl>,
}

impl RustParser {
    pub fn new() -> RustParser {
        RustParser { scope_offset: None, ctx: Context::new() }
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
                    (FailIf(OneOf(b"\r\nbu")), OneChar(|_| true)).p(&t[i + 1..])
                {
                    return (i + 1, self.ctx.push(state, StateEl::Invalid), len, state);
                }
            }
            i += 1;
        }
        (0, state, i, state)
    }
}

impl Parser for RustParser {
    fn has_offset(&mut self) -> bool {
        self.scope_offset.is_some()
    }

    fn set_scope_offset(&mut self, offset: u32) {
        if !self.has_offset() {
            self.scope_offset = Some(offset)
        }
    }

    fn get_all_scopes(&self) -> Vec<Vec<String>> {
        ALL_SCOPES
            .iter()
            .map(|stack| stack.iter().map(|s| (*s).to_string()).collect::<Vec<_>>())
            .collect()
    }

    fn get_scope_id_for_state(&self, state: State) -> ScopeId {
        let offset = self.scope_offset.unwrap_or_default();

        if let Some(element) = self.ctx.tos(state) {
            element.scope_id() + offset
        } else {
            offset
        }
    }

    fn parse(&mut self, text: &str, mut state: State) -> (usize, State, usize, State) {
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
            _ => (),
        }
        let mut i = 0;
        while i < t.len() {
            let b = t[i];
            if let Some(len) = "/*".p(&t[i..]) {
                state = self.ctx.push(state, StateEl::Comment);
                return (i, state, len, state);
            } else if "//".p(&t[i..]).is_some() {
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
            } else if let Some(len) = whitespace.p(&t[i..]) {
                return (i, self.ctx.push(state, StateEl::Source), len, state);
            }

            i += 1;
        }

        (0, self.ctx.push(state, StateEl::Source), t.len(), state)
    }
}

fn is_digit(c: u8) -> bool {
    (b'0'..=b'9').contains(&c)
}

fn is_hex_digit(c: u8) -> bool {
    (b'0'..=b'9').contains(&c) || (b'a'..=b'f').contains(&c) || (b'A'..=b'F').contains(&c)
}

// Note: will have to rework this if we want to support non-ASCII identifiers
fn is_ident_start(c: u8) -> bool {
    (b'A'..=b'Z').contains(&c) || (b'a'..=b'z').contains(&c) || c == b'_'
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
        Optional(int_suffix),
    )
        .p(s)
}

fn positive_decimal(s: &[u8]) -> Option<usize> {
    (
        raw_numeric,
        Alt(
            int_suffix,
            (
                Optional((b'.', FailIf(OneByte(is_ident_start)), Optional(raw_numeric))),
                Optional((Alt(b'e', b'E'), Optional(Alt(b'+', b'-')), raw_numeric)),
                Optional(Alt("f32", "f64")),
            ),
        ),
    )
        .p(s)
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
            ("u{", Repeat(OneByte(is_hex_digit), 1..7), b'}'),
        ),
    )
        .p(s)
}

fn char_literal(s: &[u8]) -> Option<usize> {
    (b'\'', Alt(OneChar(|c| c != '\\' && c != '\''), escape), b'\'').p(s)
}

// Parser for an arbitrary number of whitespace characters
// Reference: https://en.cppreference.com/w/cpp/string/byte/isspace
fn whitespace(s: &[u8]) -> Option<usize> {
    // 0x0B -> \v
    // 0x0C -> \f
    (OneOrMore(OneOf(&[b' ', b'\t', b'\n', b'\r', 0x0B, 0x0C]))).p(s)
}

// A simple stdio based harness for testing.
pub fn test() {
    let mut buf = String::new();
    let _ = stdin().read_to_string(&mut buf).unwrap();
    let mut c = RustParser::new();

    let mut state = State::default();
    for line in buf.lines() {
        let mut i = 0;
        while i < line.len() {
            let (prevlen, s0, len, s1) = c.parse(&line[i..], state);
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
