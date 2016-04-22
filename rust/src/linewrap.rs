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

//! Compute line wrapping breaks for text.

use time;

// TODO: figure out how not to duplcate this
macro_rules! print_err {
    ($($arg:tt)*) => (
        {
            use std::io::prelude::*;
            if let Err(e) = write!(&mut ::std::io::stderr(), "{}\n", format_args!($($arg)*)) {
                panic!("Failed to write to stderr.\
                    \nOriginal error output: {}\
                    \nSecondary error writing to stderr: {}", format!($($arg)*), e);
            }
        }
    )
}

use xi_rope::rope::{Rope, RopeInfo};
use xi_rope::tree::Cursor;
use xi_rope::breaks::{Breaks, BreakBuilder};
use xi_unicode::LineBreakLeafIter;

struct LineBreakCursor<'a> {
    inner: Cursor<'a, RopeInfo>,
    lb_iter: LineBreakLeafIter,
}

impl<'a> LineBreakCursor<'a> {
    fn new(text: &'a Rope, pos: usize) -> LineBreakCursor<'a> {
        let inner = Cursor::new(text, pos);
        let lb_iter = match inner.get_leaf() {
            Some((s, offset)) if !s.is_empty() =>
                LineBreakLeafIter::new(s.as_str(), offset),
            _ => LineBreakLeafIter::default()
        };
        LineBreakCursor {
            inner: inner,
            lb_iter: lb_iter,
        }
    }

    // position and whether break is hard
    fn next(&mut self) -> Option<(usize, bool)> {
        let mut leaf = self.inner.get_leaf();
        loop {
            match leaf {
                Some((s, offset)) => {
                    let (next, hard) = self.lb_iter.next(s.as_str());
                    if next < s.len() {
                        return Some((self.inner.pos() - offset + next, hard));
                    }
                    leaf = self.inner.next_leaf();
                }
                None => return None
            }
        }
    }
}

pub fn linewrap(text: &Rope, cols: usize) -> Breaks {
    let start_time = time::now();
    let mut wb_cursor = LineBreakCursor::new(text, 0);
    let mut builder = BreakBuilder::new();
    let mut last_pos = 0;
    let mut last_break_pos = 0;
    let mut width = 0;
    loop {
        if let Some((pos, hard)) = wb_cursor.next() {
            let word_width = pos - last_pos;
            if width > 0 && width + word_width > cols {
                builder.add_break(width);
                //print_err!("soft break {}", width);
                last_break_pos += width;
                width = 0;
            }
            width += word_width;
            if hard {
                builder.add_break(width);
                //print_err!("hard break {}", width);
                last_break_pos += width;
                width = 0;
            }
            last_pos = pos;
        } else {
            break;
        }
    }
    builder.add_no_break(text.len() - last_break_pos);
    print_err!("last {}", text.len() - last_break_pos);
    let result = builder.build();
    {
        let c = Cursor::new(&result, 0);
        print_err!("{:?}", c.get_leaf());
    }
    let time_ms = (time::now() - start_time).num_nanoseconds().unwrap() as f64 * 1e-6;
    print_err!("time to wrap {} bytes: {:.1}ms", text.len(), time_ms);
    result
}
