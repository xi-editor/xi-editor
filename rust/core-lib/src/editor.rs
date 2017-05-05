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
use std::path::{Path, PathBuf};
use std::io::Write;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use serde_json::{self, Value};

use xi_rope::rope::{LinesMetric, Rope};
use xi_rope::interval::Interval;
use xi_rope::delta::{Delta, Transformer};
use xi_rope::tree::Cursor;
use xi_rope::engine::Engine;
use xi_rope::spans::{Spans, SpansBuilder};
use view::{Style, View};
use word_boundaries::WordCursor;
use movement::Movement;

use tabs::{ViewIdentifier, TabCtx};
use rpc::{EditCommand, GestureType};
use run_plugin::{start_plugin, PluginRef, UpdateResponse, PluginEdit};

const FLAG_SELECT: u64 = 2;

const MAX_UNDOS: usize = 20;

const TAB_SIZE: usize = 4;

// Maximum returned result from plugin get_data RPC.
const MAX_SIZE_LIMIT: usize = 1024 * 1024;

pub struct Editor<W: Write> {
    text: Rope,
    // Maybe this should be in TabCtx or equivelant?
    path: Option<PathBuf>,

    /// A collection of non-primary views attached to this buffer.
    views: BTreeMap<ViewIdentifier, View>,
    engine: Engine,
    last_rev_id: usize,
    pristine_rev_id: usize,
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

    scroll_to: Option<usize>,

    style_spans: Spans<Style>,
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
    /// Creates a new `Editor` with a new empty buffer.
    pub fn new(tab_ctx: TabCtx<W>, initial_view_id: ViewIdentifier) -> Arc<Mutex<Editor<W>>> {
        Self::with_text(tab_ctx, initial_view_id, "".to_owned())
    }

    /// Creates a new `Editor`, loading text into a new buffer.
    pub fn with_text(tab_ctx: TabCtx<W>, initial_view_id: ViewIdentifier, text: String) -> Arc<Mutex<Editor<W>>> {

        let engine = Engine::new(Rope::from(text));
        let buffer = engine.get_head();
        let last_rev_id = engine.get_head_rev_id();
        let mut views = BTreeMap::new();
        views.insert(initial_view_id.clone(), View::new(initial_view_id));

        let editor = Editor {
            text: buffer,
            path: None,
            views: views,
            engine: engine,
            last_rev_id: last_rev_id,
            pristine_rev_id: last_rev_id,
            undo_group_id: 0,
            live_undos: Vec::new(),
            cur_undo: 0,
            undos: BTreeSet::new(),
            gc_undos: BTreeSet::new(),
            last_edit_type: EditType::Other,
            this_edit_type: EditType::Other,
            new_cursor: None,
            scroll_to: Some(0),
            style_spans: Spans::default(),
            tab_ctx: tab_ctx,
            plugins: Vec::new(),
            revs_in_flight: 0,
        };
        Arc::new(Mutex::new(editor))
    }


    pub fn add_view(&mut self, view_id: ViewIdentifier) {
        assert!(!self.views.contains_key(&view_id), "view_id already exists");
        self.views.insert(view_id.to_owned(), View::new(view_id.to_owned()));
    }

    /// Removes a view from this editor's stack, if this editor has multiple views.
    ///
    /// If the editor only has a single view this is a no-op. After removing a view the caller must
    /// always call Editor::has_views() to determine whether or not the editor should be cleaned up.
    #[allow(unreachable_code)]
    pub fn remove_view(&mut self, view_id: &ViewIdentifier) {
        self.views.remove(view_id).expect("attempt to remove missing view");
    }

    /// Returns true if this editor has additional attached views.
    pub fn has_views(&self) -> bool {
        self.views.len() > 0
    }

    pub fn set_path<P: AsRef<Path>>(&mut self, path: P) {
        self.path = Some(path.as_ref().to_owned());
    }

    pub fn get_path(&self) -> Option<&Path> {
        match self.path {
            Some(ref p) => Some(p),
            None => None,
        }
    }

    fn insert(&mut self, view_id: &ViewIdentifier, s: &str) {
        let (sel_interval, new_cursor) = {
            let view = self.views.get_mut(view_id).expect(&format!("Failed to get view by id {}", view_id));
            let sel_interval = Interval::new_closed_open(view.sel_min(), view.sel_max());
            let new_cursor = view.sel_min() + s.len();
            (sel_interval, new_cursor)
        };
        self.add_delta(sel_interval, Rope::from(s), new_cursor, new_cursor);
    }

    /// Sets the position of the cursor to `offset`, as part of an edit operation.
    /// If this cursor position's horizontal component was chosen implicitly
    /// (e.g. if the user moved up from the end of a long line to a shorter line)
    /// then `hard` is false. In all other cases, `hard` is true.
    fn set_cursor(&mut self, view_id: &ViewIdentifier, offset: usize, hard: bool) {
        let view = self.views.get_mut(view_id).expect(&format!("Failed to get view by id {}", view_id));
        let edit_type = self.this_edit_type;
        if edit_type != EditType::Select {
            view.sel_start = offset;
        }
        view.sel_end = offset;
        if hard {
            let new_col = view.offset_to_line_col(&self.text, offset).1;
            view.set_cursor_col(new_col);
            self.scroll_to = Some(offset);
        }
        view.scroll_to_cursor(&self.text);
        view.set_old_sel_dirty();
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
    fn commit_delta(&mut self, self_ref: &Arc<Mutex<Editor<W>>>, author: Option<&str>) {
        if self.engine.get_head_rev_id() != self.last_rev_id {
            self.update_after_revision(self_ref, author);
            if let Some((start, end)) = self.new_cursor.take() {
                let keys = self.views.keys().cloned().collect::<Vec<_>>();
                // TODO: Should this update all views?
                for view_id in keys {
                    self.set_cursor(&view_id, end, true);
                    let view = self.views.get_mut(&view_id).expect(&format!("Failed to get view by id {}", view_id));
                    view.sel_start = start;
                }
            }
        }
    }

    // generates a delta from a plugin's response and applies it to the buffer.
    fn apply_plugin_edit(&mut self, self_ref: &Arc<Mutex<Editor<W>>>,
                         edit: PluginEdit, undo_group: usize) {
        let interval = Interval::new_closed_open(edit.start as usize, edit.end as usize);
        let text = Rope::from(&edit.text);
        let rev_len = self.engine.get_rev(edit.rev as usize).unwrap().len();
        let delta = Delta::simple_edit(interval, text, rev_len);
        let prev_head_rev_id = self.engine.get_head_rev_id();
        //self.engine.edit_rev(0x100000, undo_group, edit.rev as usize, delta);
        self.engine.edit_rev(edit.priority as usize, undo_group, edit.rev as usize, delta);
        self.text = self.engine.get_head();

        // adjust cursor position so that the cursor is not moved by the plugin edit
        let (changed_interval, _) = self.engine.delta_rev_head(prev_head_rev_id).summary();
        for (_, ref mut view) in &mut self.views {
            if edit.after_cursor && (changed_interval.start() as usize) == view.sel_end {
                self.new_cursor = Some((view.sel_start, view.sel_end));
            }
        }

        self.commit_delta(self_ref, Some(&edit.author));
        self.render();
    }

    fn update_undos(&mut self, self_ref: &Arc<Mutex<Editor<W>>>) {
        self.engine.undo(self.undos.clone());
        self.text = self.engine.get_head();
        self.update_after_revision(self_ref, None);
    }

    fn update_after_revision(&mut self, self_ref: &Arc<Mutex<Editor<W>>>, author: Option<&str>) {
        let delta = self.engine.delta_rev_head(self.last_rev_id);
        let is_pristine = self.is_pristine();
        for (_, ref mut view) in &mut self.views {
            view.after_edit(&self.text, &delta, is_pristine);
        }
        let (iv, new_len) = delta.summary();

        // TODO: maybe more precise editing based on actual delta rather than summary.
        // TODO: perhaps use different semantics for spans that enclose the edited region.
        // Currently it breaks any such span in half and applies no spans to the inserted
        // text. That's ok for syntax highlighting but not ideal for rich text.
        let empty_spans = SpansBuilder::new(new_len).build();
        self.style_spans.edit(iv, empty_spans);

        // TODO: Not sure what we should do here if no author is sent
        let author = author.unwrap_or(&"");
        let undo_group = *self.live_undos.last().unwrap();

        //print_err!("delta {}:{} +{} {}", iv.start(), iv.end(), new_len, self.this_edit_type.json_string());
        for plugin in &self.plugins {
            self.revs_in_flight += 1;
            let editor = Arc::downgrade(self_ref);
            let text = if new_len < MAX_SIZE_LIMIT {
                Some(self.text.slice_to_string(iv.start(), iv.start() + new_len))
            } else {
                None
            };
            plugin.update(iv.start(), iv.end(), new_len,
                          text.as_ref().map(|s| s.as_str()),
                          self.engine.get_head_rev_id(),
                          self.this_edit_type.json_string(), author,
                          move |response| {
                              if let Some(editor) = editor.upgrade() {
                                  let response = response.expect("bad plugin response");
                                  match serde_json::from_value::<UpdateResponse>(response) {
                                      Ok(UpdateResponse::Edit(edit)) => {
                                          //print_err!("got response {:?}", edit);
                                          editor.lock().unwrap().apply_plugin_edit(&editor, edit, undo_group);
                                      }
                                      Ok(UpdateResponse::Ack(_)) => (),
                                      Err(err) => { print_err!("plugin response json err: {:?}", err); }
                    };
                    editor.lock().unwrap().dec_revs_in_flight();
                }
            });
        }
        self.last_rev_id = self.engine.get_head_rev_id();
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

    fn is_pristine(&self) -> bool {
        self.engine.is_equivalent_revision(self.pristine_rev_id, self.engine.get_head_rev_id())
    }

    // render if needed, sending to ui
    pub fn render(&mut self) {
        for (_, ref mut view) in &mut self.views {
            view.render_if_dirty(&self.text, &self.tab_ctx, &self.style_spans);
            if let Some(scrollto) = self.scroll_to {
                let (line, col) = view.offset_to_line_col(&self.text, scrollto);
                self.tab_ctx.scroll_to(&view.view_id, line, col);
                self.scroll_to = None;
            }
        }
    }

    fn delete_forward(&mut self, view_id: &ViewIdentifier) {
        let (sel_start, sel_end) = {
            let view = self.views.get_mut(view_id).expect(&format!("Failed to get view by id {}", view_id));
            (view.sel_start, view.sel_end)
        };

        if sel_start == sel_end {
            let offset =
                if let Some(pos) = self.text.next_grapheme_offset(sel_end) {
                    pos
                } else {
                    return;
                };

            self.set_cursor(&view_id, offset, true);
        }

        self.delete(view_id);
    }

    fn delete_backward(&mut self, view_id: &ViewIdentifier) {
        self.delete(view_id);
    }

    fn delete_to_beginning_of_line(&mut self, view_id: &ViewIdentifier) {
        self.move_to_left_end_of_line(view_id, FLAG_SELECT);

        self.delete(view_id);
    }

    fn delete(&mut self, view_id: &ViewIdentifier) {
        let (sel_min, sel_max, sel_start, sel_end) = {
            let view = self.views.get_mut(view_id).expect(&format!("Failed to get view by id {}", view_id));
            (view.sel_min(), view.sel_max(), view.sel_start, view.sel_end)
        };
        let start = if sel_start != sel_end {
            sel_min
        } else if let Some(bsp_pos) = self.text.prev_codepoint_offset(sel_end) {
            // TODO: implement complex emoji logic
            bsp_pos
        } else {
            sel_max
        };

        if start < sel_max {
            self.this_edit_type = EditType::Delete;
            let del_interval = Interval::new_closed_open(start, sel_max);
            self.add_delta(del_interval, Rope::from(""), start, start);
        }
    }

    fn insert_newline(&mut self, view_id: &ViewIdentifier) {
        self.this_edit_type = EditType::InsertChars;
        self.insert(view_id, "\n");
    }

    fn insert_tab(&mut self, view_id: &ViewIdentifier) {
        let (sel_min, sel_max, sel_start, sel_end) = {
            let view = self.views.get(view_id).expect(&format!("Failed to get view by id {}", view_id));
            (view.sel_min(), view.sel_max(), view.sel_start, view.sel_end)
        };

        self.this_edit_type = EditType::InsertChars;
        if sel_start == sel_end {
            let col = {
                let view = self.views.get(view_id).expect(&format!("Failed to get view by id {}", view_id));
                let (_, col) = view.offset_to_line_col(&self.text, sel_end);
                col
            };
            let n = TAB_SIZE - (col % TAB_SIZE);
            self.insert(view_id, n_spaces(n));
        } else {
            let (first_line, last_line, last_col) = {
                let view = self.views.get(view_id).expect(&format!("Failed to get view by id {}", view_id));
                let (first_line, _) = view.offset_to_line_col(&self.text, sel_min);
                let (last_line, last_col) = view.offset_to_line_col(&self.text, sel_max);
                (first_line, last_line, last_col)
            };
            let last_line = if last_col == 0 && last_line > first_line {
                last_line
            } else {
                last_line + 1
            };
            let added = (last_line - first_line) * TAB_SIZE;
            let (start, end) = if sel_start < sel_end {
                (sel_start + TAB_SIZE, sel_end + added)
            } else {
                (sel_start + added, sel_end + TAB_SIZE)
            };
            for line in first_line..last_line {
                let offset = {
                    let view = self.views.get(view_id).expect(&format!("Failed to get view by id {}", view_id));
                    view.line_col_to_offset(&self.text, line, 0)
                };
                let iv = Interval::new_closed_open(offset, offset);
                self.add_delta(iv, Rope::from(n_spaces(TAB_SIZE)), start, end);
            }
        }
    }

    fn modify_selection(&mut self) {
        self.this_edit_type = EditType::Select;
    }

    /// Apply a movement, also setting the scroll to the point requested by
    /// the movement.
    ///
    /// The type of the `flags` parameter is a convenience to old-style
    /// movement methods.
    fn do_move(&mut self, view_id: &ViewIdentifier, movement: Movement, flags: u64) {
        let view = self.views.get_mut(view_id).expect(&format!("Failed to get view by id {}", view_id));
        self.scroll_to = view.do_move(&self.text, movement, (flags & FLAG_SELECT) != 0);
    }

    fn move_up(&mut self, view_id: &ViewIdentifier, flags: u64) {
        self.do_move(view_id, Movement::Up, flags);
    }

    fn move_down(&mut self, view_id: &ViewIdentifier, flags: u64) {
        self.do_move(view_id, Movement::Down, flags);
    }

    fn move_left(&mut self, view_id: &ViewIdentifier, flags: u64) {
        self.do_move(view_id, Movement::Left, flags);
    }

    fn move_word_left(&mut self, view_id: &ViewIdentifier, flags: u64) {
        self.do_move(view_id, Movement::LeftWord, flags);
    }

    fn move_to_left_end_of_line(&mut self, view_id: &ViewIdentifier, flags: u64) {
        self.do_move(view_id, Movement::LeftOfLine, flags);
    }

    fn move_right(&mut self, view_id: &ViewIdentifier, flags: u64) {
        self.do_move(view_id, Movement::Right, flags);
    }

    fn move_word_right(&mut self, view_id: &ViewIdentifier, flags: u64) {
        self.do_move(view_id, Movement::RightWord, flags);
    }

    fn move_to_right_end_of_line(&mut self, view_id: &ViewIdentifier, flags: u64) {
        self.do_move(view_id, Movement::RightOfLine, flags);
    }

    fn move_to_beginning_of_paragraph(&mut self, view_id: &ViewIdentifier, flags: u64) {
        self.do_move(view_id, Movement::StartOfParagraph, flags);
    }

    fn move_to_end_of_paragraph(&mut self, view_id: &ViewIdentifier, flags: u64) {
        self.do_move(view_id, Movement::EndOfParagraph, flags);
    }

    fn end_of_paragraph_offset(&mut self, view_id: &ViewIdentifier) -> usize {
        let view = self.views.get_mut(view_id).expect(&format!("Failed to get view by id {}", view_id));
        let current = view.sel_max();
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

    fn move_to_beginning_of_document(&mut self, view_id: &ViewIdentifier, flags: u64) {
        self.do_move(view_id, Movement::StartOfDocument, flags);
    }

    fn move_to_end_of_document(&mut self, view_id: &ViewIdentifier, flags: u64) {
        self.do_move(view_id, Movement::EndOfDocument, flags);
    }

    fn scroll_page_up(&mut self, view_id: &ViewIdentifier, flags: u64) {
        self.do_move(view_id, Movement::UpPage, flags);
    }

    fn scroll_page_down(&mut self, view_id: &ViewIdentifier, flags: u64) {
        self.do_move(view_id, Movement::DownPage, flags);
    }

    fn select_all(&mut self, view_id: &ViewIdentifier) {
        let view = self.views.get_mut(view_id).expect(&format!("Failed to get view by id {}", view_id));
        view.sel_start = 0;
        view.sel_end = self.text.len();
        view.set_old_sel_dirty();
    }

    fn do_key(&mut self, view_id: &ViewIdentifier, chars: &str, flags: u64) {
        match chars {
            "\r" => self.insert_newline(view_id),
            "\x7f" => {
                self.delete_backward(view_id);
            }
            "\u{F700}" => {
                // up arrow
                self.move_up(view_id, flags);
            }
            "\u{F701}" => {
                // down arrow
                self.move_down(view_id, flags);
            }
            "\u{F702}" => {
                // left arrow
                self.move_left(view_id, flags);
            }
            "\u{F703}" => {
                // right arrow
                self.move_right(view_id, flags);
            }
            "\u{F72C}" => {
                // page up
                self.scroll_page_up(view_id, flags);
            }
            "\u{F72D}" => {
                // page down
                self.scroll_page_down(view_id, flags);
            }
            "\u{F704}" => {
                // F1, but using for debugging
                self.debug_rewrap(view_id);
            }
            "\u{F705}" => {
                // F2, but using for debugging
                self.debug_test_fg_spans(view_id);
            }
            _ => self.insert(view_id, chars),
        }
    }

    // TODO: insert from keyboard or input method shouldn't break undo group,
    // but paste should.
    fn do_insert(&mut self, view_id: &ViewIdentifier, chars: &str) {
        self.this_edit_type = EditType::InsertChars;
        self.insert(view_id, chars);
    }

    pub fn do_save<P: AsRef<Path>>(&mut self, path: P) {
        match File::create(&path) {
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
        // TODO: we should probably bubble up this error now. in the meantime always set path,
        // because the caller has updated the open_files list
        self.path = Some(path.as_ref().to_owned());
        self.pristine_rev_id = self.last_rev_id;
        for (_, ref mut view) in &mut self.views {
            view.set_pristine();
            view.set_dirty();
        }
        self.render();
    }

    fn do_scroll(&mut self, view_id: &ViewIdentifier, first: i64, last: i64) {
        let view = self.views.get_mut(view_id).expect(&format!("Failed to get view by id {}", view_id));
        let first = max(first, 0) as usize;
        let last = last as usize;
        view.set_scroll(first, last);
        view.send_update_for_scroll(&self.text, &self.tab_ctx, &self.style_spans, first, last);
    }

    /// Sets the cursor and scrolls to the beginning of the given line.
    fn do_goto_line(&mut self, view_id: &ViewIdentifier, line: u64) {
        let line = {
            let view = self.views.get_mut(view_id).expect(&format!("Failed to get view by id {}", view_id));
            view.line_col_to_offset(&self.text, line as usize, 0)
        };
        self.set_cursor(&view_id, line, true);
    }

    fn do_request_lines(&mut self, view_id: &ViewIdentifier, first: i64, last: i64) {
        let view = self.views.get_mut(view_id).expect(&format!("Failed to get view by id {}", view_id));
        view.send_update(&self.text, &self.tab_ctx, &self.style_spans, first as usize, last as usize);
    }

    fn do_click(&mut self, view_id: &ViewIdentifier, line: u64, col: u64, flags: u64, click_count: u64) {
        let (offset, hard, modify_selection) = {
            let view = self.views.get_mut(view_id).expect(&format!("Failed to get view by id {}", view_id));
            let offset = view.line_col_to_offset(&self.text, line as usize, col as usize);
            if (flags & FLAG_SELECT) != 0 {
                (offset, true, true)
            } else if click_count == 2 {
                let (start, end) = {
                    let mut word_cursor = WordCursor::new(&self.text, offset);
                    word_cursor.select_word()
                };
                view.sel_start = start;
                (end, false, true)
            } else if click_count == 3 {
                let start = view.line_col_to_offset(&self.text, line as usize, 0);
                let end = view.line_col_to_offset(&self.text, line as usize + 1, 0);
                view.sel_start = start;
                (end, false, true)
            } else {
                (offset, true, false)
            }
        };
        if modify_selection {
            self.modify_selection();
        }
        self.set_cursor(&view_id, offset, hard);
    }

    fn do_drag(&mut self, view_id: &ViewIdentifier, line: u64, col: u64, _flags: u64) {
        let offset = {
            let view = self.views.get(view_id).expect(&format!("Failed to get view by id {}", view_id));
            view.line_col_to_offset(&self.text, line as usize, col as usize)
        };
        self.modify_selection();
        self.set_cursor(&view_id, offset, true);
    }

    fn do_gesture(&mut self, view_id: &ViewIdentifier, line: u64, col: u64, ty: GestureType) {
        let view = self.views.get_mut(view_id).expect(&format!("Failed to get view by id {}", view_id));
        let offset = view.line_col_to_offset(&self.text, line as usize, col as usize);
        match ty {
            GestureType::ToggleSel => view.toggle_sel(offset),
        }
    }

    fn debug_rewrap(&mut self, view_id: &ViewIdentifier) {
        let view = self.views.get_mut(view_id).expect(&format!("Failed to get view by id {}", view_id));
        view.rewrap(&self.text, 72);
        view.set_dirty();
    }

    fn debug_test_fg_spans(&mut self, view_id: &ViewIdentifier) {
        print_err!("setting fg spans");
        let view = self.views.get_mut(view_id).expect(&format!("Failed to get view by id {}", view_id));
        let mut sb = SpansBuilder::new(15);
        let style = Style { fg: 0xffc00000, font_style: 0 };
        sb.add_span(Interval::new_closed_open(5, 10), style);
        self.style_spans = sb.build();

        // TODO: Update all views? Or just the one sending this request
        view.set_dirty();
    }

    fn debug_run_plugin(&mut self, self_ref: &Arc<Mutex<Editor<W>>>) {
        print_err!("running plugin");
        start_plugin(self_ref.clone());
    }

    pub fn on_plugin_connect(&mut self, plugin_ref: PluginRef<W>) {
        plugin_ref.init_buf(self.plugin_buf_size(), self.engine.get_head_rev_id());
        self.plugins.push(plugin_ref);
    }

    fn do_cut(&mut self, view_id: &ViewIdentifier) -> Value {
        let (sel_min, sel_max) = {
            let view = self.views.get(view_id).expect(&format!("Failed to get view by id {}", view_id));
            (view.sel_min(), view.sel_max())
        };
        if sel_min != sel_max {
            let val = self.text.slice_to_string(sel_min, sel_max);
            let del_interval = Interval::new_closed_open(sel_min, sel_max);
            self.add_delta(del_interval, Rope::from(""), sel_min, sel_min);
            Value::String(val)
        } else {
            Value::Null
        }
    }

    fn do_copy(&mut self, view_id: &ViewIdentifier) -> Value {
        let (sel_min, sel_max, sel_start, sel_end) = {
            let view = self.views.get(view_id).expect(&format!("Failed to get view by id {}", view_id));
            (view.sel_min(), view.sel_max(), view.sel_start, view.sel_end)
        };
        if sel_start != sel_end {
            let val = self.text.slice_to_string(sel_min, sel_max);
            Value::String(val)
        } else {
            Value::Null
        }
    }

    fn do_undo(&mut self, self_ref: &Arc<Mutex<Editor<W>>>) {
        if self.cur_undo > 0 {
            self.cur_undo -= 1;
            assert!(self.undos.insert(self.live_undos[self.cur_undo]));
            self.this_edit_type = EditType::Undo;
            self.update_undos(self_ref);
        }
    }

    fn do_redo(&mut self, self_ref: &Arc<Mutex<Editor<W>>>) {
        if self.cur_undo < self.live_undos.len() {
            assert!(self.undos.remove(&self.live_undos[self.cur_undo]));
            self.cur_undo += 1;
            self.this_edit_type = EditType::Redo;
            self.update_undos(self_ref);
        }
    }

    fn do_transpose(&mut self, view_id: &ViewIdentifier) {
        let sel_end = {
            let view = self.views.get(view_id).expect(&format!("Failed to get view by id {}", view_id));
            view.sel_end
        };
        let end_opt = self.text.next_grapheme_offset(sel_end);
        let start_opt = self.text.prev_grapheme_offset(sel_end);

        let end = end_opt.unwrap_or(sel_end);
        let (start, middle) = if end_opt.is_none() && start_opt.is_some() {
            // if at the very end, swap previous TWO characters (instead of ONE)
            let middle = start_opt.unwrap();
            let start = self.text.prev_grapheme_offset(middle).unwrap_or(middle);
            (start, middle)
        } else {
            (start_opt.unwrap_or(sel_end), sel_end)
        };

        let interval = Interval::new_closed_open(start, end);
        let swapped = self.text.slice_to_string(middle, end) +
                      &self.text.slice_to_string(start, middle);
        self.add_delta(interval, Rope::from(swapped), end, end);
    }

    fn delete_to_end_of_paragraph(&mut self, view_id: &ViewIdentifier) {
        let (sel_max, sel_end) = {
            let view = self.views.get_mut(view_id).expect(&format!("Failed to get view by id {}", view_id));
            (view.sel_max(), view.sel_end)
        };
        let current = sel_max;
        let offset = self.end_of_paragraph_offset(view_id);
        let mut val = String::from("");

        if current != offset {
            val = self.text.slice_to_string(current, offset);
            let del_interval = Interval::new_closed_open(current, offset);
            self.add_delta(del_interval, Rope::from(""), current, current);
        } else if let Some(grapheme_offset) = self.text.next_grapheme_offset(sel_end) {
            val = self.text.slice_to_string(current, grapheme_offset);
            let del_interval = Interval::new_closed_open(current, grapheme_offset);
            self.add_delta(del_interval, Rope::from(""), current, current)
        }

        self.tab_ctx.set_kill_ring(Rope::from(val));
    }

    fn yank(&mut self, view_id: &ViewIdentifier) {
        let kill_ring_string = self.tab_ctx.get_kill_ring();
        self.insert(view_id, &*String::from(kill_ring_string));
    }

    pub fn do_rpc(self_ref: &Arc<Mutex<Editor<W>>>, view_id: &ViewIdentifier, cmd: EditCommand) -> Option<Value> {
        self_ref.lock().unwrap().do_rpc_with_self_ref(view_id, cmd, self_ref)
    }

    fn do_rpc_with_self_ref(&mut self, view_id: &ViewIdentifier,
                  cmd: EditCommand,
                  self_ref: &Arc<Mutex<Editor<W>>>)
                  -> Option<Value> {

        use rpc::EditCommand::*;

        self.this_edit_type = EditType::Other;

        let result = match cmd {
            Key { chars, flags } => async(self.do_key(view_id, chars, flags)),
            Insert { chars } => async(self.do_insert(view_id, chars)),
            DeleteForward => async(self.delete_forward(view_id)),
            DeleteBackward => async(self.delete_backward(view_id)),
            DeleteToEndOfParagraph => async(self.delete_to_end_of_paragraph(view_id)),
            DeleteToBeginningOfLine => async(self.delete_to_beginning_of_line(view_id)),
            InsertNewline => async(self.insert_newline(view_id)),
            InsertTab => async(self.insert_tab(view_id)),
            MoveUp => async(self.move_up(view_id, 0)),
            MoveUpAndModifySelection => async(self.move_up(view_id, FLAG_SELECT)),
            MoveDown => async(self.move_down(view_id, 0)),
            MoveDownAndModifySelection => async(self.move_down(view_id, FLAG_SELECT)),
            MoveLeft => async(self.move_left(view_id, 0)),
            MoveLeftAndModifySelection => async(self.move_left(view_id, FLAG_SELECT)),
            MoveRight => async(self.move_right(view_id, 0)),
            MoveRightAndModifySelection => async(self.move_right(view_id, FLAG_SELECT)),
            MoveWordLeft => async(self.move_word_left(view_id, 0)),
            MoveWordLeftAndModifySelection => async(self.move_word_left(view_id, FLAG_SELECT)),
            MoveWordRight => async(self.move_word_right(view_id, 0)),
            MoveWordRightAndModifySelection => async(self.move_word_right(view_id, FLAG_SELECT)),
            MoveToBeginningOfParagraph => async(self.move_to_beginning_of_paragraph(view_id, 0)),
            MoveToEndOfParagraph => async(self.move_to_end_of_paragraph(view_id, 0)),
            MoveToLeftEndOfLine => async(self.move_to_left_end_of_line(view_id, 0)),
            MoveToLeftEndOfLineAndModifySelection => async(self.move_to_left_end_of_line(view_id, FLAG_SELECT)),
            MoveToRightEndOfLine => async(self.move_to_right_end_of_line(view_id, 0)),
            MoveToRightEndOfLineAndModifySelection => async(self.move_to_right_end_of_line(view_id, FLAG_SELECT)),
            MoveToBeginningOfDocument => async(self.move_to_beginning_of_document(view_id, 0)),
            MoveToBeginningOfDocumentAndModifySelection => async(self.move_to_beginning_of_document(view_id, FLAG_SELECT)),
            MoveToEndOfDocument => async(self.move_to_end_of_document(view_id, 0)),
            MoveToEndOfDocumentAndModifySelection => async(self.move_to_end_of_document(view_id, FLAG_SELECT)),
            ScrollPageUp => async(self.scroll_page_up(view_id, 0)),
            PageUpAndModifySelection => async(self.scroll_page_up(view_id, FLAG_SELECT)),
            ScrollPageDown => async(self.scroll_page_down(view_id, 0)),
            PageDownAndModifySelection => {
                async(self.scroll_page_down(view_id, FLAG_SELECT))
            }
            SelectAll => async(self.select_all(view_id)),
            Scroll { first, last } => async(self.do_scroll(view_id, first, last)),
            GotoLine { line } => async(self.do_goto_line(view_id, line)),
            RequestLines { first, last } => async(self.do_request_lines(view_id, first, last)),
            Yank => async(self.yank(view_id)),
            Transpose => async(self.do_transpose(view_id)),
            Click { line, column, flags, click_count } => {
                async(self.do_click(view_id, line, column, flags, click_count))
            }
            Drag { line, column, flags } => async(self.do_drag(view_id, line, column, flags)),
            Gesture { line, column, ty } => async(self.do_gesture(view_id, line, column, ty)),
            Undo => async(self.do_undo(self_ref)),
            Redo => async(self.do_redo(self_ref)),
            Cut => Some(self.do_cut(view_id)),
            Copy => Some(self.do_copy(view_id)),
            DebugRewrap => async(self.debug_rewrap(view_id)),
            DebugTestFgSpans => async(self.debug_test_fg_spans(view_id)),
            DebugRunPlugin => async(self.debug_run_plugin(self_ref)),
        };

        // TODO: could defer this until input quiesces - will this help?
        self.commit_delta(self_ref, None);
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
        self.style_spans.edit(Interval::new_closed_closed(start, end_offset), spans);
        for (_, ref mut view) in &mut self.views {
            view.set_dirty();
        }
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
