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

use std::borrow::Cow;
use std::cmp::{min, max};
use std::fs::File;
use std::io::{Read, Write};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use serde_json::Value;

use xi_rope::rope::{LinesMetric, Rope};
use xi_rope::interval::Interval;
use xi_rope::delta::{Delta, Transformer};
use xi_rope::tree::Cursor;
use xi_rope::engine::Engine;
use xi_rope::spans::SpansBuilder;
use view::{Style, View};

use tabs::TabCtx;
use rpc::EditCommand;
use run_plugin::{start_plugin, PluginRef};

const FLAG_SELECT: u64 = 2;

const MAX_UNDOS: usize = 20;

const TAB_SIZE: usize = 4;

// Maximum returned result from plugin get_data RPC.
const MAX_SIZE_LIMIT: usize = 1024 * 1024;

pub struct Editor<W: Write> {
    text: Rope,
    view: View,

    engine: Engine,
    last_rev_id: usize,
    undo_group_id: usize,
    live_undos: Vec<usize>, //  undo groups that may still be toggled
    cur_undo: usize, // index to live_undos, ones after this are undone
    undos: BTreeSet<usize>, // undo groups that are undone
    gc_undos: BTreeSet<usize>, // undo groups that are no longer live and should be gc'ed

    this_edit_type: EditType,
    last_edit_type: EditType,

    // update to cursor, to be committed atomically with delta
    // TODO: use for all cursor motion?
    new_cursor: Option<(usize, usize)>,

    // TODO: may not need separate view/text dirty flags
    view_dirty: bool,
    text_dirty: bool,
    scroll_to: Option<usize>,
    col: usize, // maybe this should live in view, it's similar to selection

    tab_ctx: TabCtx<W>,
    plugins: Vec<PluginRef<W>>,
    revs_in_flight: usize,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum EditType {
    Other,
    Select,
    InsertChars,
    Delete,
    Undo,
    Redo,
}

impl EditType {
    pub fn json_string(&self) -> &'static str {
        match *self {
            EditType::InsertChars => "insert",
            EditType::Delete => "delete",
            EditType::Undo => "undo",
            EditType::Redo => "redo",
            _ => "other",
        }
    }
}

impl<W: Write + Send + 'static> Editor<W> {
    pub fn new(tab_ctx: TabCtx<W>) -> Arc<Mutex<Editor<W>>> {
        let engine = Engine::new(Rope::from(""));
        let last_rev_id = engine.get_head_rev_id();
        let editor = Editor {
            text: Rope::from(""),
            view: View::new(),
            text_dirty: false,
            view_dirty: false,
            engine: engine,
            last_rev_id: last_rev_id,
            undo_group_id: 0,
            live_undos: Vec::new(),
            cur_undo: 0,
            undos: BTreeSet::new(),
            gc_undos: BTreeSet::new(),
            last_edit_type: EditType::Other,
            this_edit_type: EditType::Other,
            new_cursor: None,
            scroll_to: Some(0),
            col: 0,
            tab_ctx: tab_ctx,
            plugins: Vec::new(),
            revs_in_flight: 0,
        };
        Arc::new(Mutex::new(editor))
    }

    fn insert(&mut self, s: &str) {
        let sel_interval = Interval::new_closed_open(self.view.sel_min(), self.view.sel_max());
        let new_cursor = self.view.sel_min() + s.len();
        self.add_delta(sel_interval, Rope::from(s), new_cursor, new_cursor);
    }

    fn set_cursor(&mut self, offset: usize, hard: bool) {
        if self.this_edit_type != EditType::Select {
            self.view.sel_start = offset;
        }
        self.view.sel_end = offset;
        if hard {
            self.col = self.view.offset_to_line_col(&self.text, offset).1;
            self.scroll_to = Some(offset);
        }
        self.view.scroll_to_cursor(&self.text);
        self.view_dirty = true;
    }

    // Apply the delta to the buffer, and store the new cursor so that it gets
    // set when commit_delta is called.
    //
    // Records the delta into the CRDT engine so that it can be undone. Also contains
    // the logic for merging edits into the same undo group. At call time,
    // self.this_edit_type should be set appropriately.
    fn add_delta(&mut self, iv: Interval, new: Rope, new_start: usize, new_end: usize) {
        let delta = Delta::simple_edit(iv, new, self.text.len());
        let head_rev_id = self.engine.get_head_rev_id();
        let undo_group;

        if self.this_edit_type == self.last_edit_type &&
            self.this_edit_type != EditType::Other &&
            self.this_edit_type != EditType::Select &&
            !self.live_undos.is_empty() {

            undo_group = *self.live_undos.last().unwrap();
        } else {
            undo_group = self.undo_group_id;
            self.gc_undos.extend(&self.live_undos[self.cur_undo..]);
            self.live_undos.truncate(self.cur_undo);
            self.live_undos.push(undo_group);
            if self.live_undos.len() <= MAX_UNDOS {
                self.cur_undo += 1;
            } else {
                self.gc_undos.insert(self.live_undos.remove(0));
            }
            self.undo_group_id += 1;
        }
        self.last_edit_type = self.this_edit_type;
        let priority = 0x10000;
        self.engine.edit_rev(priority, undo_group, head_rev_id, delta);
        self.text = self.engine.get_head();
        self.new_cursor = Some((new_start, new_end));
    }

    // commit the current delta, updating views, plugins, and other invariants as needed
    fn commit_delta(&mut self, self_ref: &Arc<Mutex<Editor<W>>>) {
        if self.engine.get_head_rev_id() != self.last_rev_id {
            self.update_after_revision(self_ref);
            if let Some((start, end)) = self.new_cursor.take() {
                self.set_cursor(end, true);
                self.view.sel_start = start;
            }
        }
    }

    fn update_undos(&mut self, self_ref: &Arc<Mutex<Editor<W>>>) {
        self.engine.undo(self.undos.clone());
        self.text = self.engine.get_head();
        self.update_after_revision(self_ref);
    }

    fn update_after_revision(&mut self, self_ref: &Arc<Mutex<Editor<W>>>) {
        let delta = self.engine.delta_rev_head(self.last_rev_id);
        self.view.after_edit(&self.text, &delta);
        let (iv, new_len) = delta.summary();
        //print_err!("delta {}:{} +{} {}", iv.start(), iv.end(), new_len, self.this_edit_type.json_string());
        for plugin in &self.plugins {
            self.revs_in_flight += 1;
            let editor = Arc::downgrade(self_ref);
            let text = if new_len < MAX_SIZE_LIMIT {
                Some(self.text.slice_to_string(iv.start(), iv.start() + new_len))
            } else {
                None
            };
            plugin.update(iv.start(), iv.end(), new_len, text.as_ref().map(|s| s.as_str()),
                self.engine.get_head_rev_id(), self.this_edit_type.json_string(),
                move |_| {
                    if let Some(editor) = editor.upgrade() {
                        editor.lock().unwrap().dec_revs_in_flight();
                    }
                    //print_err!("plugin update responded");
                }
            );
        }
        self.last_rev_id = self.engine.get_head_rev_id();
        self.text_dirty = true;
    }

    // GC of CRDT engine is deferred until all plugins have acknowledged the new rev,
    // so when the ack comes back, potentially trigger GC.
    fn dec_revs_in_flight(&mut self) {
        self.revs_in_flight -= 1;
        self.gc_undos();
    }

    fn gc_undos(&mut self) {
        if self.revs_in_flight == 0 && !self.gc_undos.is_empty() {
            self.engine.gc(&self.gc_undos);
            self.undos = &self.undos - &self.gc_undos;
            self.gc_undos.clear();
        }
    }

    fn reset_contents(&mut self, new_contents: Rope) {
        self.engine = Engine::new(new_contents);
        self.text = self.engine.get_head();
        self.text_dirty = true;
        self.view.reset_breaks();
        self.set_cursor(0, true);
    }

    // render if needed, sending to ui
    fn render(&mut self) {
        if self.text_dirty || self.view_dirty {
            self.tab_ctx.update_tab(&self.view.render(&self.text, self.scroll_to));
            self.scroll_to = None;
            self.text_dirty = false;
            self.view_dirty = false;
        }
    }

    fn delete_forward(&mut self) {
        if self.view.sel_start == self.view.sel_end {
            let offset =
                if let Some(pos) = self.text.next_grapheme_offset(self.view.sel_end) {
                    pos
                } else {
                    return;
                };

            self.set_cursor(offset, true);
        }

        self.delete();
    }

    fn delete_backward(&mut self) {
        self.delete();
    }

    fn delete_to_beginning_of_line(&mut self) {
        self.move_to_left_end_of_line(FLAG_SELECT);

        self.delete();
    }

    fn delete(&mut self) {
        let start = if self.view.sel_start != self.view.sel_end {
            self.view.sel_min()
        } else if let Some(bsp_pos) = self.text.prev_codepoint_offset(self.view.sel_end) {
            // TODO: implement complex emoji logic
            bsp_pos
        } else {
            self.view.sel_max()
        };

        if start < self.view.sel_max() {
            self.this_edit_type = EditType::Delete;
            let del_interval = Interval::new_closed_open(start, self.view.sel_max());
            self.add_delta(del_interval, Rope::from(""), start, start);
        }
    }

    fn insert_newline(&mut self) {
        self.this_edit_type = EditType::InsertChars;
        self.insert("\n");
    }

    fn insert_tab(&mut self) {
        self.this_edit_type = EditType::InsertChars;
        if self.view.sel_start == self.view.sel_end {
            let (_, col) = self.view.offset_to_line_col(&self.text, self.view.sel_end);
            let n = TAB_SIZE - (col % TAB_SIZE);
            self.insert(n_spaces(n));
        } else {
            let (first_line, _) = self.view.offset_to_line_col(&self.text, self.view.sel_min());
            let (last_line, last_col) =
                self.view.offset_to_line_col(&self.text, self.view.sel_max());
            let last_line = if last_col == 0 && last_line > first_line {
                last_line
            } else {
                last_line + 1
            };
            let added = (last_line - first_line) * TAB_SIZE;
            let (start, end) = if self.view.sel_start < self.view.sel_end {
                (self.view.sel_start + TAB_SIZE, self.view.sel_end + added)
            } else {
                (self.view.sel_start + added, self.view.sel_end + TAB_SIZE)
            };
            for line in first_line..last_line {
                let offset = self.view.line_col_to_offset(&self.text, line, 0);
                let iv = Interval::new_closed_open(offset, offset);
                self.add_delta(iv, Rope::from(n_spaces(TAB_SIZE)), start, end);
            }
        }
    }

    fn modify_selection(&mut self) {
        self.this_edit_type = EditType::Select;
    }

    fn move_up(&mut self, flags: u64) {
        if (flags & FLAG_SELECT) != 0 {
            self.modify_selection();
        }

        let old_offset = self.view.sel_end;
        let offset = self.view.vertical_motion(&self.text, -1, self.col);
        self.set_cursor(offset, old_offset == offset);
        self.scroll_to = Some(offset);
    }

    fn move_down(&mut self, flags: u64) {
        if (flags & FLAG_SELECT) != 0 {
            self.modify_selection();
        }

        let old_offset = self.view.sel_end;
        let offset = self.view.vertical_motion(&self.text, 1, self.col);
        self.set_cursor(offset, old_offset == offset);
        self.scroll_to = Some(offset);
    }

    fn move_left(&mut self, flags: u64) {
        if (flags & FLAG_SELECT) != 0 {
            self.modify_selection();
        }

        // Selecting cancel
        if self.view.sel_start != self.view.sel_end && self.this_edit_type != EditType::Select {
            let offset = self.view.sel_min();
            self.set_cursor(offset, true);

            return;
        }

        // Normal move
        if let Some(offset) = self.text.prev_grapheme_offset(self.view.sel_end) {
            self.set_cursor(offset, true);
        } else {
                self.col = 0;
            // TODO: should set scroll_to_cursor in this case too,
            // but it won't get sent; probably it needs to be a separate cmd
        }
    }

    fn move_to_left_end_of_line(&mut self, flags: u64) {
        if (flags & FLAG_SELECT) != 0 {
            self.modify_selection();
        }

        let line_col = self.view.offset_to_line_col(&self.text, self.view.sel_end);
        let offset = self.view.line_col_to_offset(&self.text, line_col.0, 0);

        self.set_cursor(offset, true);

        return;
    }

    fn move_right(&mut self, flags: u64) {
        if (flags & FLAG_SELECT) != 0 {
            self.modify_selection();
        }

        // Selecting cancel
        if self.view.sel_start != self.view.sel_end && self.this_edit_type != EditType::Select {
            let offset = self.view.sel_max();
            self.set_cursor(offset, true);

            return;
        }

        // Normal move
        if let Some(offset) = self.text.next_grapheme_offset(self.view.sel_end) {
            self.set_cursor(offset, true);
        } else {
            self.col = self.view.offset_to_line_col(&self.text, self.view.sel_end).1;
            // see above
        }
    }

    fn move_to_right_end_of_line(&mut self, flags: u64) {
        if (flags & FLAG_SELECT) != 0 {
            self.modify_selection();
        }

        let line_col = self.view.offset_to_line_col(&self.text, self.view.sel_end);
        let mut offset = self.text.len();

        // calculate end of line
        let next_line_offset = self.view.line_col_to_offset(&self.text, line_col.0 + 1, 0);
        if offset > next_line_offset {
            if let Some(prev) = self.text.prev_grapheme_offset(next_line_offset) {
                offset = prev;
            }
        }

        self.set_cursor(offset, true);

        return;
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

    fn move_to_beginning_of_document(&mut self, flags: u64) {
        if (flags & FLAG_SELECT) != 0 {
            self.modify_selection();
        }

        let offset = 0;

        self.set_cursor(offset, true);
    }

    fn move_to_end_of_document(&mut self, flags: u64) {
        if (flags & FLAG_SELECT) != 0 {
            self.modify_selection();
        }

        let offset = self.text.len();

        self.set_cursor(offset, true);
    }

    fn scroll_page_up(&mut self, flags: u64) {
        if (flags & FLAG_SELECT) != 0 {
            self.modify_selection();
        }

        let scroll = -max(self.view.scroll_height() as isize - 2, 1);
        let old_offset = self.view.sel_end;
        let offset = self.view.vertical_motion(&self.text, scroll, self.col);
        self.set_cursor(offset, old_offset == offset);
        let scroll_offset = self.view.vertical_motion(&self.text, scroll, self.col);
        self.scroll_to = Some(scroll_offset);
    }

    fn scroll_page_down(&mut self, flags: u64) {
        if (flags & FLAG_SELECT) != 0 {
            self.modify_selection();
        }

        let scroll = max(self.view.scroll_height() as isize - 2, 1);
        let old_offset = self.view.sel_end;
        let offset = self.view.vertical_motion(&self.text, scroll, self.col);
        self.set_cursor(offset, old_offset == offset);
        let scroll_offset = self.view.vertical_motion(&self.text, scroll, self.col);
        self.scroll_to = Some(scroll_offset);
    }

    fn do_key(&mut self, chars: &str, flags: u64) {
        match chars {
            "\r" => self.insert_newline(),
            "\x7f" => {
                self.delete_backward();
            }
            "\u{F700}" => {
                // up arrow
                self.move_up(flags);
            }
            "\u{F701}" => {
                // down arrow
                self.move_down(flags);
            }
            "\u{F702}" => {
                // left arrow
                self.move_left(flags);
            }
            "\u{F703}" => {
                // right arrow
                self.move_right(flags);
            }
            "\u{F72C}" => {
                // page up
                self.scroll_page_up(flags);
            }
            "\u{F72D}" => {
                // page down
                self.scroll_page_down(flags);
            }
            "\u{F704}" => {
                // F1, but using for debugging
                self.debug_rewrap();
            }
            "\u{F705}" => {
                // F2, but using for debugging
                self.debug_test_fg_spans();
            }
            _ => self.insert(chars),
        }
    }

    // TODO: insert from keyboard or input method shouldn't break undo group,
    // but paste should.
    fn do_insert(&mut self, chars: &str) {
        self.this_edit_type = EditType::InsertChars;
        self.insert(chars);
    }

    fn do_open(&mut self, path: &str) {
        match File::open(path) {
            Ok(mut f) => {
                let mut s = String::new();
                if f.read_to_string(&mut s).is_ok() {
                    self.reset_contents(Rope::from(s));
                }
            }
            Err(e) => print_err!("error {}", e),
        }
    }

    fn do_save(&mut self, path: &str) {
        match File::create(path) {
            Ok(mut f) => {
                for chunk in self.text.iter_chunks(0, self.text.len()) {
                    if let Err(e) = f.write_all(chunk.as_bytes()) {
                        print_err!("write error {}", e);
                        break;
                    }
                }
            }
            Err(e) => print_err!("create error {}", e),
        }
    }

    fn do_scroll(&mut self, first: i64, last: i64) {
        self.view.set_scroll(max(first, 0) as usize, last as usize);
    }

    fn do_click(&mut self, line: u64, col: u64, flags: u64, _click_count: u64) {
        let offset = self.view.line_col_to_offset(&self.text, line as usize, col as usize);
        if (flags & FLAG_SELECT) != 0 {
            self.modify_selection();
        }
        self.set_cursor(offset, true);
    }

    fn do_drag(&mut self, line: u64, col: u64, _flags: u64) {
        let offset = self.view.line_col_to_offset(&self.text, line as usize, col as usize);
        self.modify_selection();
        self.set_cursor(offset, true);
    }

    fn do_render_lines(&mut self, first_line: usize, last_line: usize) -> Value {
        self.this_edit_type = self.last_edit_type;  // doesn't break undo group
        self.view.render_lines(&self.text, first_line as usize, last_line as usize)
    }

    fn debug_rewrap(&mut self) {
        self.view.rewrap(&self.text, 72);
        self.view_dirty = true;
    }

    fn debug_test_fg_spans(&mut self) {
        print_err!("setting fg spans");
        self.view.set_test_fg_spans();
        self.view_dirty = true;
    }

    fn debug_run_plugin(&mut self, self_ref: &Arc<Mutex<Editor<W>>>) {
        print_err!("running plugin");
        start_plugin(self_ref.clone());
    }

    pub fn on_plugin_connect(&mut self, plugin_ref: PluginRef<W>) {
        plugin_ref.init_buf(self.plugin_buf_size(), self.engine.get_head_rev_id());
        self.plugins.push(plugin_ref);
    }

    fn do_cut(&mut self) -> Value {
        let min = self.view.sel_min();
        if min != self.view.sel_max() {
            let val = self.text.slice_to_string(min, self.view.sel_max());
            let del_interval = Interval::new_closed_open(min, self.view.sel_max());
            self.add_delta(del_interval, Rope::from(""), min, min);
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

    fn do_undo(&mut self, self_ref: &Arc<Mutex<Editor<W>>>) {
        if self.cur_undo > 0 {
            self.cur_undo -= 1;
            debug_assert!(self.undos.insert(self.live_undos[self.cur_undo]));
            self.this_edit_type = EditType::Undo;
            self.update_undos(self_ref);
        }
    }

    fn do_redo(&mut self, self_ref: &Arc<Mutex<Editor<W>>>) {
        if self.cur_undo < self.live_undos.len() {
            debug_assert!(self.undos.remove(&self.live_undos[self.cur_undo]));
            self.cur_undo += 1;
            self.this_edit_type = EditType::Redo;
            self.update_undos(self_ref);
        }
    }

    fn do_transpose(&mut self) {
        let end_opt = self.text.next_grapheme_offset(self.view.sel_end);
        let start_opt = self.text.prev_grapheme_offset(self.view.sel_end);

        let end = end_opt.unwrap_or(self.view.sel_end);
        let (start, middle) = if end_opt.is_none() && start_opt.is_some() {
            // if at the very end, swap previous TWO characters (instead of ONE)
            let middle = start_opt.unwrap();
            let start = self.text.prev_grapheme_offset(middle).unwrap_or(middle);
            (start, middle)
        } else {
            (start_opt.unwrap_or(self.view.sel_end), self.view.sel_end)
        };

        let interval = Interval::new_closed_open(start, end);
        let swapped = self.text.slice_to_string(middle, end) +
                      &self.text.slice_to_string(start, middle);
        self.add_delta(interval, Rope::from(swapped), end, end);
    }

    fn delete_to_end_of_paragraph(&mut self) {
        let current = self.view.sel_max();
        let offset = self.cursor_end_offset();
        let mut val = String::from("");

        if current != offset {
            val = self.text.slice_to_string(current, offset);
            let del_interval = Interval::new_closed_open(current, offset);
            self.add_delta(del_interval, Rope::from(""), current, current);
        } else if let Some(grapheme_offset) = self.text.next_grapheme_offset(self.view.sel_end) {
            val = self.text.slice_to_string(current, grapheme_offset);
            let del_interval = Interval::new_closed_open(current, grapheme_offset);
            self.add_delta(del_interval, Rope::from(""), current, current)
        }

        self.tab_ctx.set_kill_ring(Rope::from(val));
    }

    fn yank(&mut self) {
        let kill_ring_string = self.tab_ctx.get_kill_ring();
        self.insert(&*String::from(kill_ring_string));
    }

    pub fn do_rpc(self_ref: &Arc<Mutex<Editor<W>>>, cmd: EditCommand) -> Option<Value> {
        self_ref.lock().unwrap().do_rpc_with_self_ref(cmd, self_ref)
    }

    fn do_rpc_with_self_ref(&mut self,
                  cmd: EditCommand,
                  self_ref: &Arc<Mutex<Editor<W>>>)
                  -> Option<Value> {

        use rpc::EditCommand::*;

        self.this_edit_type = EditType::Other;

        let result = match cmd {
            RenderLines { first_line, last_line } => {
                Some(self.do_render_lines(first_line, last_line))
            }
            Key { chars, flags } => async(self.do_key(chars, flags)),
            Insert { chars } => async(self.do_insert(chars)),
            DeleteForward => async(self.delete_forward()),
            DeleteBackward => async(self.delete_backward()),
            DeleteToEndOfParagraph => async(self.delete_to_end_of_paragraph()),
            DeleteToBeginningOfLine => async(self.delete_to_beginning_of_line()),
            InsertNewline => async(self.insert_newline()),
            InsertTab => async(self.insert_tab()),
            MoveUp => async(self.move_up(0)),
            MoveUpAndModifySelection => async(self.move_up(FLAG_SELECT)),
            MoveDown => async(self.move_down(0)),
            MoveDownAndModifySelection => async(self.move_down(FLAG_SELECT)),
            MoveLeft => async(self.move_left(0)),
            MoveLeftAndModifySelection => async(self.move_left(FLAG_SELECT)),
            MoveRight => async(self.move_right(0)),
            MoveRightAndModifySelection => async(self.move_right(FLAG_SELECT)),
            MoveToBeginningOfParagraph => async(self.cursor_start()),
            MoveToEndOfParagraph => async(self.cursor_end()),
            MoveToLeftEndOfLine => async(self.move_to_left_end_of_line(0)),
            MoveToLeftEndOfLineAndModifySelection => async(self.move_to_left_end_of_line(FLAG_SELECT)),
            MoveToRightEndOfLine => async(self.move_to_right_end_of_line(0)),
            MoveToRightEndOfLineAndModifySelection => async(self.move_to_right_end_of_line(FLAG_SELECT)),
            MoveToBeginningOfDocument => async(self.move_to_beginning_of_document(0)),
            MoveToBeginningOfDocumentAndModifySelection => async(self.move_to_beginning_of_document(FLAG_SELECT)),
            MoveToEndOfDocument => async(self.move_to_end_of_document(0)),
            MoveToEndOfDocumentAndModifySelection => async(self.move_to_end_of_document(FLAG_SELECT)),
            ScrollPageUp => async(self.scroll_page_up(0)),
            PageUpAndModifySelection => async(self.scroll_page_up(FLAG_SELECT)),
            ScrollPageDown => async(self.scroll_page_down(0)),
            PageDownAndModifySelection => {
                async(self.scroll_page_down(FLAG_SELECT))
            }
            Open { file_path } => async(self.do_open(file_path)),
            Save { file_path } => async(self.do_save(file_path)),
            Scroll { first, last } => async(self.do_scroll(first, last)),
            Yank => async(self.yank()),
            Transpose => async(self.do_transpose()),
            Click { line, column, flags, click_count } => {
                async(self.do_click(line, column, flags, click_count))
            }
            Drag { line, column, flags } => async(self.do_drag(line, column, flags)),
            Undo => async(self.do_undo(self_ref)),
            Redo => async(self.do_redo(self_ref)),
            Cut => Some(self.do_cut()),
            Copy => Some(self.do_copy()),
            DebugRewrap => async(self.debug_rewrap()),
            DebugTestFgSpans => async(self.debug_test_fg_spans()),
            DebugRunPlugin => async(self.debug_run_plugin(self_ref)),
        };

        // TODO: could defer this until input quiesces - will this help?
        self.commit_delta(self_ref);
        self.render();
        self.last_edit_type = self.this_edit_type;
        self.gc_undos();
        result
    }

    // Note: the following are placeholders for prototyping, and are not intended to
    // deal with asynchrony or be efficient.

    pub fn plugin_buf_size(&self) -> usize {
        self.text.len()
    }

    pub fn plugin_n_lines(&self) -> usize {
        self.text.measure::<LinesMetric>() + 1
    }

    pub fn plugin_get_line(&self, line_num: usize) -> String {
        let start_offset = self.text.offset_of_line(line_num);
        let end_offset = self.text.offset_of_line(line_num + 1);
        self.text.slice_to_string(start_offset, end_offset)
    }

    pub fn plugin_set_fg_spans(&mut self, start: usize, len: usize, spans: &Value, rev: usize) {
        // TODO: more protection against invalid input
        let mut start = start;
        let mut end_offset = start + len;
        let mut sb = SpansBuilder::new(len);
        for span in spans.as_array().unwrap() {
            let span_dict = span.as_object().unwrap();
            let start = span_dict.get("start").and_then(Value::as_u64).unwrap() as usize;
            let end = span_dict.get("end").and_then(Value::as_u64).unwrap() as usize;
            let fg = span_dict.get("fg").and_then(Value::as_u64).unwrap() as u32;
            let font_style = span_dict.get("font").and_then(Value::as_u64).unwrap_or(0) as u8;
            let style = Style { fg: fg, font_style: font_style };
            sb.add_span(Interval::new_open_open(start, end), style);
        }
        let mut spans = sb.build();
        if rev != self.engine.get_head_rev_id() {
            let delta = self.engine.delta_rev_head(rev);
            let mut transformer = Transformer::new(&delta);
            let new_start = transformer.transform(start, false);
            if !transformer.interval_untouched(Interval::new_closed_closed(start, end_offset)) {
                spans = spans.transform(start, end_offset, &mut transformer);
            }
            start = new_start;
            end_offset = transformer.transform(end_offset, true);
        }
        self.view.set_fg_spans(start, end_offset, spans);
        self.view_dirty = true;
        self.render();
    }

    pub fn plugin_get_data(&self, offset: usize, max_size: usize, rev: usize) -> Option<String> {
        let text_cow = if rev == self.engine.get_head_rev_id() {
            Cow::Borrowed(&self.text)
        } else {
            match self.engine.get_rev(rev) {
                None => return None,
                Some(text) => Cow::Owned(text)
            }
        };
        let text = &text_cow;
        // Enforce start is on codepoint boundary.
        if !text.is_codepoint_boundary(offset) { return None; }
        let max_size = min(max_size, MAX_SIZE_LIMIT);
        let mut end_off = offset.saturating_add(max_size);
        if end_off >= text.len() {
            end_off = text.len();
        } else {
            // Snap end to codepoint boundary.
            end_off = text.prev_codepoint_offset(end_off + 1).unwrap();
        }
        Some(text.slice_to_string(offset, end_off))
    }

    // Note: currently we route up through Editor to TabCtx, but perhaps the plugin
    // should have its own reference.
    pub fn plugin_alert(&self, msg: &str) {
        self.tab_ctx.alert(msg);
    }
}

// wrapper so async methods don't have to return None themselves
fn async(_: ()) -> Option<Value> {
    None
}

fn n_spaces(n: usize) -> &'static str {
    let spaces = "                                ";
    assert!(n <= spaces.len());
    &spaces[..n]
}
