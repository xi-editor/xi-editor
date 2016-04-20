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

//! Run line break test data.

// Run on:
// http://www.unicode.org/Public/UCD/latest/ucd/auxiliary/LineBreakTest.txt
// or use randomized data from tools/gen_rand_icu.cc (same format)

extern crate xi_unicode;

use xi_unicode::LineBreakIterator;

use std::io::prelude::*;
use std::io::BufReader;
use std::fs::File;

fn quote_str(s: &str) -> String {
    let mut result = String::new();
    for c in s.chars() {
        if c == '"' || c == '\\' {
            result.push('\\');
        }
        if ' ' <= c && c <= '~' {
            result.push(c);
        } else {
            result.push_str(&format!("\\u{{{:04x}}}", c as u32));
        }
    }
    result
}

fn check_breaks(s: &str, breaks: &[usize]) -> bool {
    let my_breaks = LineBreakIterator::new(s)
        .map(|(bk, _hard)| bk)
        .collect::<Vec<_>>();
    if my_breaks != breaks {
        println!("failed case: \"{}\"", quote_str(s));
        println!("expected {:?} actual {:?}", breaks, my_breaks);
        return false;
    }
    true
}

fn run_test(filename: &str) -> std::io::Result<()> {
    let f = try!(File::open(filename));
    let mut reader = BufReader::new(f);
    let mut pass = 0;
    let mut total = 0;
    loop {
        let mut line = String::new();
        if try!(reader.read_line(&mut line)) == 0 { break };
        let mut s = String::new();
        let mut breaks = Vec::new();
        for token in line.split_whitespace() {
            if token == "÷" {
                breaks.push(s.len());
            } else if token == "×" {
            } else if token == "#" {
                break;
            } else if let Ok(cp) = u32::from_str_radix(token, 16) {
                s.push(std::char::from_u32(cp).unwrap());
            }
        }
        total += 1;
        if check_breaks(&s, &breaks) { pass += 1; }
    }
    println!("{}/{} pass", pass, total);
    Ok(())
}

fn main() {
    let mut args = std::env::args();
    let _ = args.next();
    let _ = run_test(&args.next().unwrap());
}
