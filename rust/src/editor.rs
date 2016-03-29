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

use std::fs::File;
use std::io::Read;
use serde_json::Value;

use dimer_rope::Rope;
use view::View;

use ::send;

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

const MODIFIER_SHIFT: u64 = 2;

pub struct Editor {
	text: Rope,
    view: View,
    dirty: bool,
    col: usize  // maybe this should live in view, it's similar to selection
}

impl Editor {
	pub fn new() -> Editor {
		Editor {
			text: Rope::from(""),
            view: View::new(),
            dirty: false,
            col: 0
		}
	}

    fn insert(&mut self, s: &str) {
        self.text.edit_str(self.view.sel_min(), self.view.sel_max(), s);
        let new_cursor = self.view.sel_min() + s.len();
        self.set_cursor(new_cursor, true);
    }

    fn set_cursor_impl(&mut self, offset: usize, set_start: bool, hard: bool) {
        if set_start {
            self.view.sel_start = offset;
        }
        self.view.sel_end = offset;
        if hard {
            self.col = self.view.offset_to_line_col(&self.text, offset).1;
        }
        self.view.scroll_to_cursor(&self.text);
        self.dirty = true;
    }

    fn set_cursor(&mut self, offset: usize, hard: bool) {
        self.set_cursor_impl(offset, true, hard);
    }

    // set the cursor or update the selection, depending on the flags
    fn set_cursor_or_sel(&mut self, offset: usize, flags: u64, hard: bool) {
        self.set_cursor_impl(offset, flags & MODIFIER_SHIFT == 0, hard);
    }

    // render if needed, sending to ui
    fn render(&mut self) {
        if self.dirty {
            send(&self.view.render(&self.text, 10));
            self.dirty = false;
        }
    }

    fn do_key(&mut self, args: &Value) {
        if let Some(args) = args.as_object() {
            let chars = args.get("chars").unwrap().as_string().unwrap();
            let flags = args.get("flags").unwrap().as_u64().unwrap();
            match chars {
                "\r" => self.insert("\n"),
                "\x7f" => {
                    let start = if self.view.sel_start != self.view.sel_end {
                        self.view.sel_min()
                    } else {
                        if let Some(bsp_pos) = self.text.prev_codepoint_offset(self.view.sel_end) {
                        // TODO: implement complex emoji logic
                            bsp_pos
                       } else {
                            self.view.sel_max()
                        }
                    };
                    if start < self.view.sel_max() {
                        self.text.edit_str(start, self.view.sel_max(), "");
                        self.set_cursor(start, true);
                    }
                },
                "\u{F700}" => {  // up arrow
                    if self.view.sel_end == 0 { return; }
                    let (line, _) = self.view.offset_to_line_col(&self.text, self.view.sel_end);
                    let offset = if line == 0 { 0 } else {
                        self.view.line_col_to_offset(&self.text, line - 1, self.col)
                    };
                    self.set_cursor_or_sel(offset, flags, false);
                },
                "\u{F701}" => {  // down arrow
                    if self.view.sel_end == self.text.len() { return; }
                    let (line, _) = self.view.offset_to_line_col(&self.text, self.view.sel_end);
                    let offset = self.view.line_col_to_offset(&self.text, line + 1, self.col);
                    self.set_cursor_or_sel(offset, flags, false);
                },
                "\u{F702}" => {  // left arrow
                    if self.view.sel_start != self.view.sel_end && (flags & MODIFIER_SHIFT) == 0 {
                        let offset = self.view.sel_min();
                        self.set_cursor(offset, true);
                    } else {
                        if let Some(offset) = self.text.prev_grapheme_offset(self.view.sel_end) {
                            self.set_cursor_or_sel(offset, flags, true);
                        }
                    }
                },
                "\u{F703}" => {  // right arrow
                    if self.view.sel_start != self.view.sel_end && (flags & MODIFIER_SHIFT) == 0 {
                        let offset = self.view.sel_max();
                        self.set_cursor(offset, true);
                    } else {
                        if let Some(offset) = self.text.next_grapheme_offset(self.view.sel_end) {
                            self.set_cursor_or_sel(offset, flags, true);
                        }
                    }
                },
                _ => self.insert(chars)
            }
        }
    }

    fn do_open(&mut self, args: &Value) {
        if let Some(path) = args.as_string() {
            match File::open(&path) {
                Ok(mut f) => {
                    let mut s = String::new();
                    if f.read_to_string(&mut s).is_ok() {
                        self.text = Rope::from(s);
                        self.set_cursor(0, true);
                    }
                },
                Err(e) => print_err!("error {}", e)
            }
        }
    }

	pub fn do_cmd(&mut self, cmd: &str, args: &Value) {
        match cmd {
		  "key" => self.do_key(args),
          "open" => self.do_open(args),
          _ => print_err!("unknown cmd {}", cmd)
        }
        self.render();
	}
}
