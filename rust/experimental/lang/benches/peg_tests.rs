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

//! Benchmarks of PEG parsing libraries

#![feature(test)]

/// Run as:
/// ```
/// run nightly cargo bench --features "nom regex pom"
/// ```
use std::env;

extern crate xi_lang;

#[cfg(test)]
extern crate test;

#[cfg(feature = "pom")]
extern crate pom;

#[cfg(feature = "regex")]
extern crate regex;

#[cfg(feature = "nom")]
#[macro_use]
extern crate nom;

#[cfg(feature = "combine")]
extern crate combine;

const TEST_STR: &str = "1.2345e56";

#[cfg(all(test, feature = "pom"))]
mod pom_benches {
    use super::{test, TEST_STR};
    use pom::parser::{one_of, sym};
    use pom::{DataInput, Parser};
    use test::Bencher;

    fn pom_number() -> Parser<u8, usize> {
        let integer = one_of(b"123456789") - one_of(b"0123456789").repeat(0..) | sym(b'0');
        let frac = sym(b'.') + one_of(b"0123456789").repeat(1..);
        let exp = one_of(b"eE") + one_of(b"+-").opt() + one_of(b"0123456789").repeat(1..);
        let number = sym(b'-').opt() + integer + frac.opt() + exp.opt();
        number.pos()
    }

    #[bench]
    fn bench_pom(b: &mut Bencher) {
        let parser = pom_number();

        b.iter(|| {
            let mut buf = DataInput::new(test::black_box(TEST_STR.as_bytes()));
            parser.parse(&mut buf)
        })
    }
}

#[cfg(all(test, feature = "regex"))]
mod regex_benches {
    use super::{test, TEST_STR};
    use regex::Regex;
    use test::Bencher;

    #[bench]
    fn bench_regex(b: &mut Bencher) {
        let re = Regex::new(r"^(0|[1-9][0-9]*)(\.[0-9]+)?([eE]([+-])?[0-9]+)?").unwrap();
        b.iter(|| re.find(test::black_box(TEST_STR)))
    }
}

#[cfg(all(test, feature = "nom"))]
mod nom_benches {
    use super::{test, TEST_STR};
    use nom::digit;
    use test::Bencher;

    named!(digits<()>, fold_many1!(digit, (), |_, _| ()));

    named!(
        nom_num<()>,
        do_parse!(
            opt!(char!('-'))
                >> alt!(map!(char!('0'), |_| ()) | digits)
                >> opt!(do_parse!(char!('.') >> digits >> ()))
                >> opt!(do_parse!(
                    alt!(char!('e') | char!('E'))
                        >> opt!(alt!(char!('+') | char!('-')))
                        >> digits
                        >> ()
                ))
                >> ()
        )
    );

    #[cfg(feature = "nom")]
    #[bench]
    fn bench_nom(b: &mut Bencher) {
        b.iter(|| nom_num(test::black_box(TEST_STR.as_bytes())))
    }
}

#[cfg(all(test, feature = "combine"))]
mod combine_benches {
    use super::{is_digit, test, TEST_STR};
    use combine::range::take_while1;
    use combine::*;
    use test::Bencher;

    fn my_number(s: &[u8]) -> ParseResult<(), &[u8]> {
        (
            token(b'-').map(Some).or(value(None)),
            token(b'0').map(|_| &b"0"[..]).or(take_while1(is_digit)),
            optional((token(b'.'), take_while1(is_digit))),
            optional((
                token(b'e').or(token(b'E')),
                token(b'-').map(Some).or(token(b'+').map(Some)).or(value(None)),
                take_while1(is_digit),
            )),
        )
            .map(|_| ())
            .parse_stream(s)
    }

    #[bench]
    fn bench_combine(b: &mut Bencher) {
        assert_eq!(parser(my_number).parse(TEST_STR.as_bytes()), Ok(((), &b""[..])));
        b.iter(|| parser(my_number).parse(test::black_box(TEST_STR.as_bytes())))
    }
}

use xi_lang::peg::{Alt, OneByte, OneOrMore, Optional, Peg};

fn is_digit(c: u8) -> bool {
    (b'0'..=b'9').contains(&c)
}

fn my_number(s: &[u8]) -> Option<usize> {
    (
        Optional('-'),
        Alt('0', OneOrMore(OneByte(is_digit))),
        Optional(('.', OneOrMore(OneByte(is_digit)))),
        Optional((Alt('e', 'E'), Optional(Alt('-', '+')), OneOrMore(OneByte(is_digit)))),
    )
        .p(s)
}

fn main() {
    if let Some(s) = env::args().nth(1) {
        println!("my: {:?}", my_number(s.as_bytes()));
        /*
        let mut buf = DataInput::new(s.as_bytes());
        println!("pom: {:?}", pom_number().parse(&mut buf));
        let re = Regex::new(r"^(0|[1-9][0-9]*)(\.[0-9]+)?([eE]([+-])?[0-9]+)?").unwrap();
        println!("regex: {:?}", re.find(&s));
        println!("nom: {:?}", nom_num(s.as_bytes()));
        */
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test::Bencher;

    #[bench]
    fn bench_my_peg(b: &mut Bencher) {
        b.iter(|| my_number(test::black_box(TEST_STR.as_bytes())))
    }
}
