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
extern crate xi_unicode;

use std::fs::File;
use std::io::BufReader;
use std::io::Read;

use xi_unicode::LineBreakIterator;

const TEST_FILE: &str = "tests/LineBreakTest.txt";

#[test]
fn line_break_test() {
    let file = File::open(TEST_FILE).expect("unable to open test file.");

    let mut reader = BufReader::new(file);
    let mut buffer = String::new();

    reader.read_to_string(&mut buffer).expect("failed to read test file.");

    let mut failed_tests = Vec::new();

    for full_test in buffer.lines().filter(|s| !s.starts_with('#')) {
        let test = full_test.split('#').next().unwrap().trim();

        let (string, breaks) = parse_test(test);
        let xi_lb = LineBreakIterator::new(&string).map(|(idx, _)| idx).collect::<Vec<_>>();

        if xi_lb != breaks {
            failed_tests.push((full_test.to_string(), breaks, xi_lb));
        }
    }

    if !failed_tests.is_empty() {
        println!("\nFailed {} line break tests.", failed_tests.len());

        for fail in failed_tests {
            println!("Failed Test:    {}", fail.0);
            println!("Unicode Breaks: {:?}", fail.1);
            println!("Xi Breaks:      {:?}\n", fail.2);
        }

        panic!("failed line break test.");
    }
}

// A typical test looks like: "× 0023 × 0308 × 0020 ÷ 0023 ÷"
fn parse_test(test: &str) -> (String, Vec<usize>) {
    use std::char;

    let mut parts = test.split(' ');
    let mut idx = 0usize;

    let mut string = String::new();
    let mut breaks = Vec::new();

    loop {
        let next = parts.next();
        if next.is_none() {
            break;
        }

        if next != Some("×") && next != Some("÷") {
            panic!("syntax error");
        }

        if next == Some("÷") {
            breaks.push(idx);
        }

        if let Some(hex) = parts.next() {
            let num = u32::from_str_radix(hex, 16).expect("syntax error");
            let ch = char::from_u32(num).expect("invalid codepoint");
            string.push(ch);
            idx += ch.len_utf8();
        } else {
            break;
        }
    }

    (string, breaks)
}
