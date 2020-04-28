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
#![feature(test)]

extern crate test;

#[cfg(test)]
mod bench {
    use std::cmp::max;
    use test::{black_box, Bencher};
    use xi_unicode::linebreak_property;
    use xi_unicode::linebreak_property_str;
    use xi_unicode::LineBreakIterator;

    fn linebreak_property_chars(s: &str) -> u8 {
        linebreak_property(black_box(s).chars().next().unwrap())
    }

    // compute the maximum numeric value of the lb, a model for iterating a string
    fn max_lb_chars(s: &str) -> u8 {
        let mut result = 0;
        for c in s.chars() {
            result = max(result, linebreak_property(c))
        }
        result
    }

    fn max_lb(s: &str) -> u8 {
        let mut result = 0;
        let mut ix = 0;
        while ix < s.len() {
            let (lb, len) = linebreak_property_str(s, ix);
            result = max(result, lb);
            ix += len;
        }
        result
    }

    #[bench]
    fn linebreak_lo(b: &mut Bencher) {
        b.iter(|| linebreak_property(black_box('\u{0042}')));
    }

    #[bench]
    fn linebreak_lo2(b: &mut Bencher) {
        b.iter(|| linebreak_property(black_box('\u{0644}')));
    }

    #[bench]
    fn linebreak_med(b: &mut Bencher) {
        b.iter(|| linebreak_property(black_box('\u{200D}')));
    }

    #[bench]
    fn linebreak_hi(b: &mut Bencher) {
        b.iter(|| linebreak_property(black_box('\u{1F680}')));
    }

    #[bench]
    fn linebreak_str_lo(b: &mut Bencher) {
        b.iter(|| linebreak_property_str("\\u{0042}", 0));
    }

    #[bench]
    fn linebreak_str_lo2(b: &mut Bencher) {
        b.iter(|| linebreak_property_str("\\u{0644}", 0));
    }

    #[bench]
    fn linebreak_str_med(b: &mut Bencher) {
        b.iter(|| linebreak_property_str("\\u{200D}", 0));
    }

    #[bench]
    fn linebreak_str_hi(b: &mut Bencher) {
        b.iter(|| linebreak_property_str("\u{1F680}", 0));
    }

    #[bench]
    fn linebreak_chars_lo2(b: &mut Bencher) {
        b.iter(|| linebreak_property_chars("\\u{0644}"));
    }

    #[bench]
    fn linebreak_chars_hi(b: &mut Bencher) {
        b.iter(|| linebreak_property_chars("\\u{1F680}"));
    }

    #[bench]
    fn max_lb_chars_hi(b: &mut Bencher) {
        b.iter(|| max_lb_chars("\\u{1F680}\\u{1F680}\\u{1F680}\\u{1F680}\\u{1F680}\\u{1F680}\\u{1F680}\\u{1F680}\\u{1F680}\\u{1F680}"));
    }

    #[bench]
    fn max_lb_hi(b: &mut Bencher) {
        b.iter(|| max_lb("\\u{1F680}\\u{1F680}\\u{1F680}\\u{1F680}\\u{1F680}\\u{1F680}\\u{1F680}\\u{1F680}\\u{1F680}\\u{1F680}"));
    }

    #[bench]
    fn max_lb_lo(b: &mut Bencher) {
        b.iter(|| max_lb("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"));
    }

    #[bench]
    fn max_lb_chars_lo(b: &mut Bencher) {
        b.iter(|| max_lb_chars("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"));
    }

    #[bench]
    fn lb_iter(b: &mut Bencher) {
        // 73 ASCII characters
        let s = "Now is the time for all good persons to come to the aid of their country.";
        b.iter(|| LineBreakIterator::new(s).count())
    }
}
