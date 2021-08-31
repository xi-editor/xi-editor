// Copyright 2016 The xi-editor Authors.
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

use xi_unicode::{LineBreakIterator, LineBreakLeafIter};

use std::fs::File;
use std::io::prelude::*;
use std::io::BufReader;

fn quote_str(s: &str) -> String {
    let mut result = String::new();
    for c in s.chars() {
        if c == '"' || c == '\\' {
            result.push('\\');
        }
        if (' '..='~').contains(&c) {
            result.push(c);
        } else {
            result.push_str(&format!("\\u{{{:04x}}}", c as u32));
        }
    }
    result
}

fn check_breaks(s: &str, breaks: &[usize]) -> bool {
    let my_breaks = LineBreakIterator::new(s).map(|(bk, _hard)| bk).collect::<Vec<_>>();
    if my_breaks != breaks {
        println!("failed case: \"{}\"", quote_str(s));
        println!("expected {:?} actual {:?}", breaks, my_breaks);
        return false;
    }
    true
}

// Verify that starting iteration at a break is insensitive to look-behind.
fn check_lb(s: &str) -> bool {
    let breaks = LineBreakIterator::new(s).collect::<Vec<_>>();
    for i in 0..breaks.len() - 1 {
        let mut cursor = LineBreakLeafIter::new(s, breaks[i].0);
        for &bk in &breaks[i + 1..] {
            let mut next = cursor.next(s);
            if next.0 == s.len() {
                next = (s.len(), true);
            }
            if next != bk {
                println!("failed case: \"{}\"", quote_str(s));
                println!("expected {:?} actual {:?}", bk, next);
                return false;
            }
        }
    }
    true
}

fn run_test(filename: &str, lb: bool) -> std::io::Result<()> {
    let f = File::open(filename)?;
    let mut reader = BufReader::new(f);
    let mut pass = 0;
    let mut total = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        };
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
        if lb {
            if check_lb(&s) {
                pass += 1;
            }
        } else if check_breaks(&s, &breaks) {
            pass += 1;
        }
    }
    println!("{}/{} pass", pass, total);
    Ok(())
}

fn main() {
    let mut args = std::env::args();
    let _ = args.next();
    let filename = args.next().unwrap();
    match args.next() {
        None => {
            let _ = run_test(&filename, false);
        }
        Some(ref s) if s == "--lookbehind" => {
            let _ = run_test(&filename, true);
        }
        _ => {
            println!("unknown argument");
        }
    }
}
