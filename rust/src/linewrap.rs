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

struct WordBreakCursor<'a> {
    inner: Cursor<'a, RopeInfo>,
}

impl<'a> WordBreakCursor<'a> {
    fn new(text: &'a Rope, pos: usize) -> WordBreakCursor<'a> {
        WordBreakCursor {
            inner: Cursor::new(text, pos)
        }
    }

    // position and whether break is hard
    fn next(&mut self) -> Option<(usize, bool)> {
        let mut last_was_space = false;
        loop {
            let pos = self.inner.pos();
            if let Some(c) = self.inner.next_codepoint() {
                match c {
                    '\n' => return Some((self.inner.pos(), true)),
                    ' ' => last_was_space = true,
                    _ if last_was_space => return Some((pos, false)),
                    _ => ()
                }
            } else {
                return None;
            }
        }
    }
}

pub fn linewrap(text: &Rope, cols: usize) -> Breaks {
    let mut wb_cursor = WordBreakCursor::new(text, 0);
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
    result
}
