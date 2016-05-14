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
use std::sync::Mutex;
use serde_json::Value;

use xi_rope::rope::{LinesMetric,Rope,RopeInfo};
use xi_rope::interval::Interval;
use xi_rope::delta::Delta;
use xi_rope::tree::Cursor;
use xi_rope::engine::Engine;
use view::View;

use tabs::update_tab;

const MODIFIER_SHIFT: u64 = 2;

pub struct Editor {
    tabname: String,  // used for sending updates back to front-end

    text: Rope,
    view: View,
    delta: Option<Delta<RopeInfo>>,

    engine: Engine,

    // update to cursor, to be committed atomically with delta
    // TODO: use for all cursor motion?
    new_cursor: Option<usize>,

    dirty: bool,
    scroll_to: Option<usize>,
    col: usize  // maybe this should live in view, it's similar to selection
}

impl Editor {
    pub fn new(tabname: &str) -> Editor {
        Editor {
            tabname: tabname.to_string(),
            text: Rope::from(""),
            view: View::new(),
            dirty: false,
            delta: None,
            engine: Engine::new(Rope::from("")),
            new_cursor: None,
            scroll_to: Some(0),
            col: 0
        }
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
        if self.delta.is_some() {
            print_err!("not supporting multiple deltas, dropping change");
            return;
        }
        self.delta = Some(Delta::simple_edit(iv, new, self.text.len()));
        self.new_cursor = Some(new_cursor);
    }

    // commit the current delta, updating views and other invariants as needed
    fn commit_delta(&mut self) {
        if let Some(delta) = self.delta.take() {
            let head_rev_id = self.engine.get_head_rev_id();
            let undo_group = 1;  // TODO
            let priority = 0x10000;
            self.engine.edit_rev(priority, undo_group, head_rev_id, delta);
            self.update_after_revision();
            if let Some(c) = self.new_cursor.take() {
                self.set_cursor(c, true);
            }
        }
    }

    fn update_after_revision(&mut self) {
        // TODO: update view
        let delta = self.engine.delta_head();
        self.view.before_edit(&self.text, &delta);
        self.text = self.engine.get_head();
        self.view.after_edit(&self.text, &delta);
        self.dirty = true;
    }

    fn reset_contents(&mut self, new_contents: Rope) {
        self.engine = Engine::new(new_contents);
        self.text = self.engine.get_head();
        self.dirty = true;
        self.view.reset_breaks();
        self.set_cursor(0, true);
    }

    // render if needed, sending to ui
    fn render(&mut self) {
        if self.dirty {
            update_tab(&self.view.render(&self.text, self.scroll_to), &self.tabname);
            self.dirty = false;
            self.scroll_to = None;
        }
    }

    fn delete_backward(&mut self) {
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

    fn insert_newline(&mut self) {
        self.insert("\n");
    }
    
    fn move_up(&mut self, flags: u64) {
        let old_offset = self.view.sel_end;
        let offset = self.view.vertical_motion(&self.text, -1, self.col);
        self.set_cursor_or_sel(offset, flags, old_offset == offset);
        self.scroll_to = Some(offset);
    }

    fn move_down(&mut self, flags: u64) {
        let old_offset = self.view.sel_end;
        let offset = self.view.vertical_motion(&self.text, 1, self.col);
        self.set_cursor_or_sel(offset, flags, old_offset == offset);
        self.scroll_to = Some(offset);
    }

    fn move_left(&mut self, flags: u64) {
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

    fn move_right(&mut self, flags: u64) {
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

    fn cursor_start(&mut self) {
        let start = self.view.sel_min() - self.col;
        self.set_cursor(start, true);
    }

    fn cursor_end(&mut self) {
        let offset = self.cursor_end_offset();
        self.set_cursor(offset, true);
    }

    fn cursor_end_offset(&mut self) -> usize {
        let current = self.view.sel_max();
        let rope = self.text.clone();
        let mut cursor = Cursor::new(&rope, current);
        match cursor.next::<LinesMetric>() {
            None => current,
            Some(offset) => {
                if cursor.is_boundary::<LinesMetric>() {
                    if let Some(new) = rope.prev_grapheme_offset(offset) {
                        new
                    } else {
                        offset
                    }
                } else {
                    offset
                }
            }
        }
    }

    fn scroll_page_up(&mut self, flags: u64) {
        let scroll = -max(self.view.scroll_height() as isize - 2, 1);
        let old_offset = self.view.sel_end;
        let offset = self.view.vertical_motion(&self.text, scroll, self.col);
        self.set_cursor_or_sel(offset, flags, old_offset == offset);
        let scroll_offset = self.view.vertical_motion(&self.text, scroll, self.col);
        self.scroll_to = Some(scroll_offset);
    }

    fn scroll_page_down(&mut self, flags: u64) {
        let scroll = max(self.view.scroll_height() as isize - 2, 1);
        let old_offset = self.view.sel_end;
        let offset = self.view.vertical_motion(&self.text, scroll, self.col);
        self.set_cursor_or_sel(offset, flags, old_offset == offset);
        let scroll_offset = self.view.vertical_motion(&self.text, scroll, self.col);
        self.scroll_to = Some(scroll_offset);
    }

    fn do_key(&mut self, args: &Value) {
        if let Some(args) = args.as_object() {
            let chars = args.get("chars").unwrap().as_string().unwrap();
            let flags = args.get("flags").unwrap().as_u64().unwrap();
            match chars {
                "\r" => self.insert_newline(),
                "\x7f" => {
                    self.delete_backward();
                }
                "\u{F700}" => {  // up arrow
                    self.move_up(flags);
                }
                "\u{F701}" => {  // down arrow
                    self.move_down(flags);
                }
                "\u{F702}" => {  // left arrow
                    self.move_left(flags);
                }
                "\u{F703}" => {  // right arrow
                    self.move_right(flags);
                }
                "\u{F72C}" => {  // page up
                    self.scroll_page_up(flags);
                }
                "\u{F72D}" => {  // page down
                    self.scroll_page_down(flags);
                }
                "\u{F704}" => {  // F1, but using for debugging
                    self.debug_rewrap();
                }
                "\u{F705}" => {  // F2, but using for debugging
                    self.debug_test_fg_spans();
                }
                _ => self.insert(chars)
            }
        }
    }

    fn do_insert(&mut self, args: &Value) {
        if let Some(args) = args.as_object() {
            let chars = args.get("chars").unwrap().as_string().unwrap();
            self.insert(chars);
        }
    }

    fn do_open(&mut self, args: &Value) {
        if let Some(path) = args.as_object()
                .and_then(|v| v.get("filename")).and_then(|v| v.as_string()) {
            match File::open(&path) {
                Ok(mut f) => {
                    let mut s = String::new();
                    if f.read_to_string(&mut s).is_ok() {
                        self.reset_contents(Rope::from(s));
                    }
                },
                Err(e) => print_err!("error {}", e)
            }
        }
    }

    fn do_save(&mut self, args: &Value) {
        if let Some(path) = args.as_object()
                .and_then(|v| v.get("filename")).and_then(|v| v.as_string()) {
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

    fn do_click(&mut self, args: &Value) {
        if let Some(array) = args.as_array() {
            if let (Some(line), Some(col), Some(flags), Some(_click_count)) =
                    (array[0].as_u64(), array[1].as_u64(), array[2].as_u64(), array[3].as_u64()) {
                let offset = self.view.line_col_to_offset(&self.text, line as usize, col as usize);
                self.set_cursor_or_sel(offset, flags, true);
            }
        }
    }

    fn do_drag(&mut self, args: &Value) {
        if let Some(array) = args.as_array() {
            if let (Some(line), Some(col), Some(_flags)) =
                    (array[0].as_u64(), array[1].as_u64(), array[2].as_u64()) {
                let offset = self.view.line_col_to_offset(&self.text, line as usize, col as usize);
                self.set_cursor_or_sel(offset, MODIFIER_SHIFT, true);
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

    fn debug_rewrap(&mut self) {
        self.view.rewrap(&self.text, 72);
        self.dirty = true;
    }

    fn debug_test_fg_spans(&mut self) {
        print_err!("setting fg spans");
        self.view.set_test_fg_spans();
        self.dirty = true;
    }

    fn do_cut(&mut self) -> Value {
        let min = self.view.sel_min();
        if min != self.view.sel_max() {
            let del_interval = Interval::new_closed_open(min, self.view.sel_max());
            self.add_delta(del_interval, Rope::from(""), min);
            let val = self.text.slice_to_string(min, self.view.sel_max());
            Value::String(val)
        } else {
            Value::Null
        }
    }

    fn do_copy(&mut self) -> Value {
        if self.view.sel_start != self.view.sel_end {
            let val = self.text.slice_to_string(self.view.sel_min(), self.view.sel_max());
            Value::String(val)
        } else {
            Value::Null
        }
    }

    fn delete_to_end_of_paragraph(&mut self, kill_ring: &Mutex<Rope>) {
        let current = self.view.sel_max();
        let offset = self.cursor_end_offset();
        let mut val = String::from("");

        if current != offset {
            val = self.text.slice_to_string(current, offset);
            let del_interval = Interval::new_closed_open(current, offset);
            self.add_delta(del_interval, Rope::from(""), current);
        } else {
            if let Some(grapheme_offset) = self.text.next_grapheme_offset(self.view.sel_end) {
                val = self.text.slice_to_string(current, grapheme_offset);
                let del_interval = Interval::new_closed_open(current, grapheme_offset);
                self.add_delta(del_interval, Rope::from(""), current)
            }
        }

        let mut kill_ring = kill_ring.lock().unwrap();
        *kill_ring = Rope::from(val);
    }

    fn yank(&mut self, kill_ring: &Mutex<Rope>) {
        let data = kill_ring.lock().unwrap();
        self.insert(&*String::from(data.clone()));
    }

    pub fn do_rpc(&mut self, method: &str, params: &Value, kill_ring: &Mutex<Rope>) -> Option<Value> {
        let result = match method {
            "render_lines" => Some(self.do_render_lines(params)),
            "key" => async(self.do_key(params)),
            "insert" => async(self.do_insert(params)),
            "delete_backward" => async(self.delete_backward()),
            "delete_to_end_of_paragraph" => async(self.delete_to_end_of_paragraph(kill_ring)),
            "insert_newline" => async(self.insert_newline()),
            "move_up" => async(self.move_up(0)),
            "move_up_and_modify_selection" => async(self.move_up(MODIFIER_SHIFT)),
            "move_down" => async(self.move_down(0)),
            "move_down_and_modify_selection" => async(self.move_down(MODIFIER_SHIFT)),
            "move_left" |
            "move_backward" => async(self.move_left(0)),
            "move_left_and_modify_selection" => async(self.move_left(MODIFIER_SHIFT)),
            "move_right" |
            "move_forward" => async(self.move_right(0)),
            "move_right_and_modify_selection" => async(self.move_right(MODIFIER_SHIFT)),
            "move_to_beginning_of_paragraph" => async(self.cursor_start()),
            "move_to_end_of_paragraph" => async(self.cursor_end()),
            "scroll_page_up" |
            "page_up" => async(self.scroll_page_up(0)),
            "page_up_and_modify_selection" => async(self.scroll_page_up(MODIFIER_SHIFT)),
            "scroll_page_down" |
            "page_down" => async(self.scroll_page_down(0)),
            "page_down_and_modify_selection" => async(self.scroll_page_down(MODIFIER_SHIFT)),
            "open" => async(self.do_open(params)),
            "save" => async(self.do_save(params)),
            "scroll" => async(self.do_scroll(params)),
            "yank" => async(self.yank(kill_ring)),
            "click" => async(self.do_click(params)),
            "drag" => async(self.do_drag(params)),
            "cut" => Some(self.do_cut()),
            "copy" => Some(self.do_copy()),
            "debug_rewrap" => async(self.debug_rewrap()),
            "debug_test_fg_spans" => async(self.debug_test_fg_spans()),
            _ => async(print_err!("unknown method {}", method))
        };
        // TODO: could defer this until input quiesces - will this help?
        self.commit_delta();
        self.render();
        result
    }
}

// wrapper so async methods don't have to return None themselves
fn async(_: ()) -> Option<Value> {
    None
}
