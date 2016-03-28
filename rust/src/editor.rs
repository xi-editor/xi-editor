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

use serde_json::Value;

use dimer_rope::Rope;
use view::View;

use ::send;

pub struct Editor {
	text: Rope,
    view: View,
    dirty: bool
}

impl Editor {
	pub fn new() -> Editor {
		Editor {
			text: Rope::from(""),
            view: View::new(),
            dirty: false
		}
	}

    fn insert(&mut self, s: &str) {
        self.text.edit_str(self.view.sel_start, self.view.sel_end, s);
        let new_cursor = self.view.sel_start + s.len();
        self.set_cursor(new_cursor);
    }

    fn set_cursor(&mut self, offset: usize) {
        self.view.sel_start = offset;
        self.view.sel_end = offset;
        self.dirty = true;
    }

	pub fn do_cmd(&mut self, cmd: &str, args: &Value) {
		if cmd == "key" {
           	if let Some(args) = args.as_object() {
	            let chars = args.get("chars").unwrap().as_string().unwrap();
                match chars {
                    "\r" => self.insert("\n"),
                    "\x7f" => {
                        let start = if self.view.sel_start < self.view.sel_end {
                            self.view.sel_start
                        } else {
                            if let Some(bsp_pos) = self.text.prev_codepoint_offset(self.view.sel_end) {
                            // TODO: implement complex emoji logic
                                bsp_pos
                           } else {
                                self.view.sel_end
                            }
                        };
                        if start < self.view.sel_end {
                            self.text.edit_str(start, self.view.sel_end, "");
                            self.set_cursor(start);
                        }
                    },
                    "\u{F702}" => {  // left arrow
                        if let Some(offset) = self.text.prev_grapheme_offset(self.view.sel_start) {
                            self.set_cursor(offset);
                        }
                    },
                    "\u{F703}" => {  // right arrow
                        if let Some(offset) = self.text.next_grapheme_offset(self.view.sel_end) {
                            self.set_cursor(offset);
                        }
                    },
                    _ => self.insert(chars)
                }
                if self.dirty {
                    send(&self.view.render(&self.text, 10));
                    self.dirty = false;
                }
            }
        }
	}
}
