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

use std::cmp::max;
use std::fs::File;
use std::io::{Read,Write};
use serde_json::Value;
use serde_json::builder::ArrayBuilder;

use xi_rope::rope::{Rope,RopeInfo};
use xi_rope::interval::Interval;
use xi_rope::delta::Delta;
use view::View;

use ::send;

const MODIFIER_SHIFT: u64 = 2;

pub struct Editor {
    text: Rope,
    view: View,
    delta: Delta<RopeInfo>,

    // update to cursor, to be committed atomically with delta
    // TODO: use for all cursor motion?
    new_cursor: Option<usize>,

    dirty: bool,
    scroll_to: Option<usize>,
    col: usize  // maybe this should live in view, it's similar to selection
}

impl Default for Editor {
    fn default() -> Editor {
        Editor {
            text: Rope::from(""),
            view: View::new(),
            dirty: false,
            delta: Delta::new(),
            new_cursor: None,
            scroll_to: Some(0),
            col: 0
        }
    }
}

impl Editor {
    pub fn new() -> Editor {
        Editor::default()
    }

    fn insert(&mut self, s: &str) {
        let sel_interval = Interval::new_closed_open(self.view.sel_min(), self.view.sel_max());
        let new_cursor = self.view.sel_min() + s.len();
        self.add_delta(sel_interval, Rope::from(s), new_cursor);
    }

    fn set_cursor_impl(&mut self, offset: usize, set_start: bool, hard: bool) {
        if set_start {
            self.view.sel_start = offset;
        }
        self.view.sel_end = offset;
        if hard {
            self.col = self.view.offset_to_line_col(&self.text, offset).1;
            self.scroll_to = Some(offset);
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

    fn add_delta(&mut self, iv: Interval, new: Rope, new_cursor: usize) {
        self.delta.add(iv, new);
        self.new_cursor = Some(new_cursor);
    }

    // commit the current delta, updating views and other invariants as needed
    fn commit_delta(&mut self) {
        if !self.delta.is_empty() {
            self.view.before_edit(&self.text, &self.delta);
            self.delta.apply(&mut self.text);
            self.view.after_edit(&self.text, &self.delta);
            if let Some(c) = self.new_cursor {
                self.set_cursor(c, true);
                self.new_cursor = None;
            }
            self.dirty = true;
            self.delta = Delta::new();
        }
    }

    // render if needed, sending to ui
    fn render(&mut self) {
        if self.dirty {
            if let Err(e) = send(&self.view.render(&self.text, self.scroll_to)) {
                print_err!("send error in render method: {}", e);
            }
            self.dirty = false;
            self.scroll_to = None;
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
                        let del_interval = Interval::new_closed_open(start, self.view.sel_max());
                        self.add_delta(del_interval, Rope::from(""), start);
                    }
                }
                "\u{F700}" => {  // up arrow
                    let old_offset = self.view.sel_end;
                    let offset = self.view.vertical_motion(&self.text, -1, self.col);
                    self.set_cursor_or_sel(offset, flags, old_offset == offset);
                    self.scroll_to = Some(offset);
                }
                "\u{F701}" => {  // down arrow
                    let old_offset = self.view.sel_end;
                    let offset = self.view.vertical_motion(&self.text, 1, self.col);
                    self.set_cursor_or_sel(offset, flags, old_offset == offset);
                    self.scroll_to = Some(offset);
                }
                "\u{F702}" => {  // left arrow
                    if self.view.sel_start != self.view.sel_end && (flags & MODIFIER_SHIFT) == 0 {
                        let offset = self.view.sel_min();
                        self.set_cursor(offset, true);
                    } else {
                        if let Some(offset) = self.text.prev_grapheme_offset(self.view.sel_end) {
                            self.set_cursor_or_sel(offset, flags, true);
                        } else {
                            self.col = 0;
                            // TODO: should set scroll_to_cursor in this case too,
                            // but it won't get sent; probably it needs to be a separate cmd
                        }
                    }
                }
                "\u{F703}" => {  // right arrow
                    if self.view.sel_start != self.view.sel_end && (flags & MODIFIER_SHIFT) == 0 {
                        let offset = self.view.sel_max();
                        self.set_cursor(offset, true);
                    } else {
                        if let Some(offset) = self.text.next_grapheme_offset(self.view.sel_end) {
                            self.set_cursor_or_sel(offset, flags, true);
                        } else {
                            self.col = self.view.offset_to_line_col(&self.text, self.view.sel_end).1;
                            // see above
                        }
                    }
                }
                "\u{F72C}" => {  // page up
                    let scroll = -max(self.view.scroll_height() as isize - 2, 1);
                    let old_offset = self.view.sel_end;
                    let offset = self.view.vertical_motion(&self.text, scroll, self.col);
                    self.set_cursor_or_sel(offset, flags, old_offset == offset);
                    let scroll_offset = self.view.vertical_motion(&self.text, scroll, self.col);
                    self.scroll_to = Some(scroll_offset);
                }
                "\u{F72D}" => {  // page down
                    let scroll = max(self.view.scroll_height() as isize - 2, 1);
                    let old_offset = self.view.sel_end;
                    let offset = self.view.vertical_motion(&self.text, scroll, self.col);
                    self.set_cursor_or_sel(offset, flags, old_offset == offset);
                    let scroll_offset = self.view.vertical_motion(&self.text, scroll, self.col);
                    self.scroll_to = Some(scroll_offset);
                }
                "\u{F704}" => {  // F1, but using for debugging
                    self.view.rewrap(&self.text, 72);
                    self.dirty = true;
                }
                "\u{F705}" => {  // F2, but using for debugging
                    print_err!("setting fg spans");
                    self.view.set_test_fg_spans();
                    self.dirty = true;
                }
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
                        self.view.reset_breaks();
                        self.set_cursor(0, true);
                    }
                },
                Err(e) => print_err!("error {}", e)
            }
        }
    }

    fn do_save(&mut self, args: &Value) {
        if let Some(path) = args.as_string() {
            match File::create(&path) {
                Ok(mut f) => {
                    for chunk in self.text.iter_chunks(0, self.text.len()) {
                        if let Err(e) = f.write_all(chunk.as_bytes()) {
                            print_err!("write error {}", e);
                            break;
                        }
                    }
                },
                Err(e) => print_err!("create error {}", e)
            }
        }
    }

    fn do_scroll(&mut self, args: &Value) {
        if let Some(array) = args.as_array() {
            if let (Some(first), Some(last)) = (array[0].as_i64(), array[1].as_i64()) {
                self.view.set_scroll(max(first, 0) as usize, last as usize);
            }
        }
    }

    fn do_render_lines(&mut self, args: &Value) -> Value {
        if let Some(dict) = args.as_object() {
            let first_line = dict.get("first_line").unwrap().as_u64().unwrap();
            let last_line = dict.get("last_line").unwrap().as_u64().unwrap();
            self.view.render_lines(&self.text, first_line as usize, last_line as usize)
        } else {
            Value::Null
        }
    }

    fn dispatch_rpc(&mut self, cmd: &str, args: &Value) -> Value {
        match cmd {
            "render_lines" => self.do_render_lines(args),
            _ => Value::Null
        }
    }

    fn do_rpc(&mut self, args: &Value) {
        if let Some(dict) = args.as_object() {
            let index = dict.get("index").unwrap();
            let request = dict.get("request").unwrap();
            if let Some(array) = request.as_array() {
                if let Some(cmd) = array[0].as_string() {
                    let result = self.dispatch_rpc(cmd, &array[1]);
                    if let Err(e) = send(&ArrayBuilder::new()
                        .push("rpc_response")
                        .push_object(|builder|
                            builder
                                .insert("index", index)
                                .insert("result", result)
                        )
                        .unwrap()
                    ) {
                        print_err!("send error in do_rpc method: {}", e);
                    }
                }
            }
        }
    }

    pub fn do_cmd(&mut self, cmd: &str, args: &Value) {
        match cmd {
            "rpc" => self.do_rpc(args),
            "key" => self.do_key(args),
            "open" => self.do_open(args),
            "save" => self.do_save(args),
            "scroll" => self.do_scroll(args),
            _ => print_err!("unknown cmd {}", cmd)
        }
        // TODO: could defer this until input quiesces - will this help?
        self.commit_delta();
        self.render();
    }
}
