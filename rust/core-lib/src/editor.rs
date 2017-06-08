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
use std::mem;
use serde_json::Value;

use xi_rope::rope::{LinesMetric, Rope, RopeInfo};
use xi_rope::interval::Interval;
use xi_rope::delta::{self, Delta, Transformer};
use xi_rope::engine::Engine;
use xi_rope::spans::{Spans, SpansBuilder};
use view::{Style, View};
use word_boundaries::WordCursor;
use movement::{Movement, region_movement};
use selection::{Affinity, Selection, SelRegion};

use tabs::{BufferIdentifier, ViewIdentifier, DocumentCtx};
use rpc::{EditCommand, GestureType};
use syntax::SyntaxDefinition;
use plugins::rpc_types::{PluginUpdate, PluginEdit, ScopeSpan, PluginBufferInfo};
use plugins::PluginPid;
use layers::Scopes;


#[cfg(not(target_os = "fuchsia"))]
pub struct SyncStore;
#[cfg(target_os = "fuchsia")]
use fuchsia::sync::SyncStore;

const FLAG_SELECT: u64 = 2;

const MAX_UNDOS: usize = 20;

const TAB_SIZE: usize = 4;

// Maximum returned result from plugin get_data RPC.
const MAX_SIZE_LIMIT: usize = 1024 * 1024;

pub struct Editor<W: Write> {
    text: Rope,

    path: Option<PathBuf>,
    buffer_id: BufferIdentifier,
    syntax: SyntaxDefinition,

    /// A collection of non-primary views attached to this buffer.
    views: BTreeMap<ViewIdentifier, View>,
    /// The currently active view. This property is dynamically modified as events originating in
    /// different views arrive.
    view: View,
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

    scroll_to: Option<usize>,

    style_spans: Spans<Style>,
    style_scopes: Scopes,
    doc_ctx: DocumentCtx<W>,
    revs_in_flight: usize,

    /// Used only on Fuchsia for syncing
    #[allow(dead_code)]
    sync_store: Option<SyncStore>,
    #[allow(dead_code)]
    last_synced_rev: usize,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum EditType {
    Other,
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
    pub fn new(doc_ctx: DocumentCtx<W>, buffer_id: BufferIdentifier,
               initial_view_id: &ViewIdentifier) -> Editor<W> {
        Self::with_text(doc_ctx, buffer_id, initial_view_id, "".to_owned())
    }

    /// Creates a new `Editor`, loading text into a new buffer.
    pub fn with_text(doc_ctx: DocumentCtx<W>, buffer_id: BufferIdentifier,
                     initial_view_id: &ViewIdentifier, text: String) -> Editor<W> {

        let engine = Engine::new(Rope::from(text));
        let buffer = engine.get_head().clone();
        let last_rev_id = engine.get_head_rev_id();

        let editor = Editor {
            text: buffer,
            buffer_id: buffer_id,
            path: None,
            syntax: SyntaxDefinition::default(),
            views: BTreeMap::new(),
            view: View::new(initial_view_id),
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
            scroll_to: Some(0),
            style_spans: Spans::default(),
            style_scopes: Scopes::default(),
            doc_ctx: doc_ctx,
            revs_in_flight: 0,
            sync_store: None,
            last_synced_rev: last_rev_id,
        };
        editor
    }


    #[allow(unreachable_code, unused_variables)]
    pub fn add_view(&mut self, view_id: &ViewIdentifier) {
        panic!("multi-view support is not currently implemented");
        assert!(!self.views.contains_key(view_id), "view_id already exists");
        self.views.insert(view_id.to_owned(), View::new(view_id));
    }

    /// Removes a view from this editor's stack, if this editor has multiple views.
    ///
    /// If the editor only has a single view this is a no-op. After removing a view the caller must
    /// always call Editor::has_views() to determine whether or not the editor should be cleaned up.
    #[allow(unreachable_code)]
    pub fn remove_view(&mut self, view_id: &ViewIdentifier) {
        if self.view.view_id == *view_id {
            if self.views.len() > 0 {
                panic!("multi-view support is not currently implemented");
                //set some other view as active. This will be reset on the next EditCommand
                let tempkey = self.views.keys().nth(0).unwrap().clone();
                let mut temp = self.views.remove(&tempkey).unwrap();
                mem::swap(&mut temp, &mut self.view);
                self.views.insert(temp.view_id.clone(), temp);
            }
        } else {
            self.views.remove(view_id).expect("attempt to remove missing view");
        }
    }

    /// Returns true if this editor has additional attached views.
    pub fn has_views(&self) -> bool {
        self.views.len() > 0
    }

    /// should only ever be called from `BufferContainerRef::set_path`
    #[doc(hidden)]
    pub fn _set_path<P: AsRef<Path>>(&mut self, path: P) {
        let path = path.as_ref();
        //TODO: if the user sets syntax, we shouldn't overwrite here
        self.syntax = SyntaxDefinition::new(path.to_str());
        self.path = Some(path.to_owned());
    }

    /// If this `Editor`'s buffer has been saved, Returns its path.
    pub fn get_path(&self) -> Option<&Path> {
        match self.path {
            Some(ref p) => Some(p),
            None => None,
        }
    }

    /// Returns this `Editor`'s active `SyntaxDefinition`.
    pub fn get_syntax(&self) -> &SyntaxDefinition {
        &self.syntax
    }

    /// Returns this `Editor`'s `BufferIdentifier`.
    pub fn get_identifier(&self) -> BufferIdentifier {
        self.buffer_id
    }

    // each outstanding plugin edit represents a rev_in_flight.
    pub fn increment_revs_in_flight(&mut self) {
        self.revs_in_flight += 1;
    }

    // GC of CRDT engine is deferred until all plugins have acknowledged the new rev,
    // so when the ack comes back, potentially trigger GC.
    pub fn dec_revs_in_flight(&mut self) {
        self.revs_in_flight -= 1;
        self.gc_undos();
    }

    /// Returns buffer information used to initialize plugins.
    pub fn plugin_init_info(&self) -> PluginBufferInfo {
        let nb_lines = self.text.measure::<LinesMetric>() + 1;
        let mut views = self.views.keys()
            .map(|s| s.to_owned())
            .collect::<Vec<_>>();
        views.push(self.view.view_id.to_owned());
        PluginBufferInfo::new(self.buffer_id, &views,
                              self.engine.get_head_rev_id(), self.text.len(),
                              nb_lines, self.path.clone(), self.syntax.clone())
    }

    fn insert(&mut self, s: &str) {
        let rope = Rope::from(s);
        let mut builder = delta::Builder::new(self.text.len());
        for region in self.view.sel_regions() {
            let iv = Interval::new_closed_open(region.min(), region.max());
            builder.replace(iv, rope.clone());
        }
        self.add_delta(builder.build());
    }

    /// Sets the position of the cursor to `offset`, as part of an edit operation.

    // TODO: add affinity.
    fn set_cursor(&mut self, offset: usize) {
        self.set_sel_single_region(SelRegion {
            start: offset,
            end: offset,
            horiz: None,
            affinity: Affinity::default(),
        });
    }

    /// Sets the selection to a single region, and scrolls the end of that
    /// region into view.
    fn set_sel_single_region(&mut self, region: SelRegion) {
        let mut sel = Selection::new();
        sel.add_region(region);
        self.scroll_to = self.view.set_selection(&self.text, sel);
    }

    /// Applies a delta to the text, and updates undo state.
    ///
    /// Records the delta into the CRDT engine so that it can be undone. Also
    /// contains the logic for merging edits into the same undo group. At call
    /// time, self.this_edit_type should be set appropriately.
    ///
    /// This method can be called multiple times, accumulating deltas that will
    /// be committed at once with `commit_delta`. Note that it does not update
    /// the views. Thus, view-associated state such as the selection and line
    /// breaks are to be considered invalid after this method, until the
    /// `commit_delta` call.
    fn add_delta(&mut self, delta: Delta<RopeInfo>) {
        let head_rev_id = self.engine.get_head_rev_id();
        let undo_group;

        if self.this_edit_type == self.last_edit_type &&
            self.this_edit_type != EditType::Other &&
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
        self.text = self.engine.get_head().clone();
    }

    /// Commits the current delta, updating views, plugins, and other invariants as needed.
    fn commit_delta(&mut self, author: Option<&str>) {
        if self.engine.get_head_rev_id() != self.last_rev_id {
            self.update_after_revision(author);
        }
    }

    // generates a delta from a plugin's response and applies it to the buffer.
    pub fn apply_plugin_edit(&mut self, edit: PluginEdit, undo_group: usize) {
        let interval = Interval::new_closed_open(edit.start as usize, edit.end as usize);
        let text = Rope::from(&edit.text);
        let rev_len = self.engine.get_rev(edit.rev as usize).unwrap().len();
        let delta = Delta::simple_edit(interval, text, rev_len);
        //let prev_head_rev_id = self.engine.get_head_rev_id();
        self.engine.edit_rev(edit.priority as usize, undo_group, edit.rev as usize, delta);
        self.text = self.engine.get_head().clone();

        // TODO: actually implement priority, which makes the need for the following
        // logic go away.
        /*
        // adjust cursor position so that the cursor is not moved by the plugin edit
        let (changed_interval, _) = self.engine.delta_rev_head(prev_head_rev_id).summary();
        if edit.after_cursor && (changed_interval.start() as usize) == self.view.sel_end {
            self.new_cursor = Some((self.view.sel_start, self.view.sel_end));
        }
        */

        self.commit_delta(Some(&edit.author));
        self.render();
    }

    fn update_undos(&mut self) {
        self.engine.undo(self.undos.clone());
        self.text = self.engine.get_head().clone();
        self.update_after_revision(None);
    }

    fn update_after_revision(&mut self, author: Option<&str>) {
        let delta = self.engine.delta_rev_head(self.last_rev_id);
        let is_pristine = self.is_pristine();
        self.scroll_to = self.view.after_edit(&self.text, &delta, is_pristine);
        let (iv, new_len) = delta.summary();

        // TODO: maybe more precise editing based on actual delta rather than summary.
        // TODO: perhaps use different semantics for spans that enclose the edited region.
        // Currently it breaks any such span in half and applies no spans to the inserted
        // text. That's ok for syntax highlighting but not ideal for rich text.
        let empty_spans = SpansBuilder::new(new_len).build();
        self.style_scopes.update_all(iv.clone(), empty_spans);

        //TODO: style_spans should just live in style_scopes
        // or maybe just be removed or something
        let empty_spans = SpansBuilder::new(new_len).build();
        self.style_spans.edit(iv, empty_spans);


        // We increment revs in flight once here, and we decrement once
        // after sending plugin updates, regardless of whether or not any actual
        // plugins get updated. This ensures that gc runs.
        self.increment_revs_in_flight();

        {
            let author = author.unwrap_or(&self.view.view_id.as_str());
            let text = match new_len < MAX_SIZE_LIMIT {
                true => Some(self.text.slice_to_string(iv.start(), iv.start() + new_len)),
                false => None
            };

            let update = PluginUpdate::new(
                iv.start(), iv.end(), new_len,
                self.engine.get_head_rev_id(), text,
                self.this_edit_type.json_string().to_owned(),
                author.to_owned());

            let undo_group = *self.live_undos.last().unwrap_or(&0);
            let view_id = self.view.view_id.clone();
            self.doc_ctx.update_plugins(view_id, update, undo_group);
        }


        self.last_rev_id = self.engine.get_head_rev_id();
        self.sync_state_changed();
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
        self.view.render_if_dirty(&self.text, &self.doc_ctx, &self.style_spans);
        if let Some(scrollto) = self.scroll_to {
            let (line, col) = self.view.offset_to_line_col(&self.text, scrollto);
            self.doc_ctx.scroll_to(&self.view.view_id, line, col);
            self.scroll_to = None;
        }
    }

    pub fn merge_new_state(&mut self, new_engine: Engine) {
        // TODO: CRDT merge
        self.engine = new_engine;
        self.text = self.engine.get_head().clone();
        self.last_synced_rev = self.engine.get_head_rev_id();
        self.commit_delta(None);
        self.render();
    }

    #[cfg(target_os = "fuchsia")]
    pub fn set_sync_store(&mut self, sync_store: SyncStore) {
        self.sync_store = Some(sync_store);
    }

    #[cfg(not(target_os = "fuchsia"))]
    pub fn sync_state_changed(&mut self) {
    }

    #[cfg(target_os = "fuchsia")]
    pub fn sync_state_changed(&mut self) {
        if let Some(sync_store) = self.sync_store.as_mut() {
            // we don't want to sync right after recieving a new merge
            if self.last_synced_rev != self.engine.get_head_rev_id() {
                self.last_synced_rev = self.engine.get_head_rev_id();
                sync_store.state_changed();
            }
        }
    }

    #[cfg(target_os = "fuchsia")]
    pub fn transaction_ready(&mut self) {
        if let Some(sync_store) = self.sync_store.as_mut() {
            sync_store.commit_transaction(&self.engine);
        }
    }

    fn delete_forward(&mut self) {
        self.delete_by_movement(Movement::Right, false);
    }

    fn delete_to_beginning_of_line(&mut self) {
        self.delete_by_movement(Movement::LeftOfLine, false);
    }

    fn delete_backward(&mut self) {
        // TODO: this function is workable but probably overall code complexity
        // could be improved by implementing a "backspace" movement instead.
        let mut builder = delta::Builder::new(self.text.len());
        for region in self.view.sel_regions() {
            let start = if !region.is_caret() {
                region.min()
            } else {
                // TODO: implement complex emoji logic
                self.text.prev_codepoint_offset(region.end).unwrap_or(region.end)
            };
            let iv = Interval::new_closed_open(start, region.max());
            if !iv.is_empty() {
                builder.delete(iv);
            }
        }

        if !builder.is_empty() {
            self.this_edit_type = EditType::Delete;
            self.add_delta(builder.build());
        }
    }

    /// Common logic for a number of delete methods. For each region in the selection,
    /// if the selection is a caret, delete the region between the caret and the
    /// movement applied to the caret, otherwise delete the region.
    ///
    /// If `save` is set, save the deleted text into the kill ring.
    fn delete_by_movement(&mut self, movement: Movement, save: bool) {
        // We compute deletions as a selection because the merge logic is convenient.
        // Another possibility would be to make the delta builder be able to handle
        // overlapping deletions (using union semantics).
        let mut deletions = Selection::new();
        for r in self.view.sel_regions() {
            if r.is_caret() {
                let new_region = region_movement(movement, r, &self.view, &self.text, true);
                deletions.add_region(new_region);
            } else {
                deletions.add_region(r.clone());
            }
        }
        if save {
            let saved = self.extract_sel_regions(&deletions).unwrap_or(String::new());
            self.doc_ctx.set_kill_ring(Rope::from(saved));
        }
        self.delete_sel_regions(&deletions);
    }

    /// Deletes the given regions.
    fn delete_sel_regions(&mut self, sel_regions: &[SelRegion]) {
        let mut builder = delta::Builder::new(self.text.len());
        for region in sel_regions {
            let iv = Interval::new_closed_open(region.min(), region.max());
            if !iv.is_empty() {
                builder.delete(iv);
            }
        }
        if !builder.is_empty() {
            self.this_edit_type = EditType::Delete;
            self.add_delta(builder.build());
        }
    }

    /// Extracts non-caret selection regions into a string, joining multiple regions
    /// with newlines.
    fn extract_sel_regions(&self, sel_regions: &[SelRegion]) -> Option<String> {
        let mut saved = None;
        for region in sel_regions {
            if !region.is_caret() {
                let val = self.text.slice_to_string(region.min(), region.max());
                match saved {
                    None => saved = Some(val),
                    Some(ref mut s) => {
                        s.push('\n');
                        s.push_str(&val);
                    }
                }
            }
        }
        saved
    }

    fn insert_newline(&mut self) {
        self.this_edit_type = EditType::InsertChars;
        self.insert("\n");
    }

    fn insert_tab(&mut self) {
        let mut builder = delta::Builder::new(self.text.len());
        for region in self.view.sel_regions() {
            let iv = Interval::new_closed_open(region.min(), region.max());
            let (_, col) = self.view.offset_to_line_col(&self.text, region.start);
            let n = TAB_SIZE - (col % TAB_SIZE);
            builder.replace(iv, Rope::from(n_spaces(n)));
        }
        self.this_edit_type = EditType::InsertChars;
        self.add_delta(builder.build());

        // What follows is old indent code, retained because it will be useful for
        // indent action (Sublime no longer does indent on non-caret selections).
        /*
            let (first_line, _) = self.view.offset_to_line_col(&self.text, self.view.sel_min());
            let (last_line, last_col) =
                self.view.offset_to_line_col(&self.text, self.view.sel_max());
            let last_line = if last_col == 0 && last_line > first_line {
                last_line
            } else {
                last_line + 1
            };
            for line in first_line..last_line {
                let offset = self.view.line_col_to_offset(&self.text, line, 0);
                let iv = Interval::new_closed_open(offset, offset);
                self.add_simple_edit(iv, Rope::from(n_spaces(TAB_SIZE)));
            }
        */
    }

    /// Apply a movement, also setting the scroll to the point requested by
    /// the movement.
    ///
    /// The type of the `flags` parameter is a convenience to old-style
    /// movement methods.
    fn do_move(&mut self, movement: Movement, flags: u64) {
        self.scroll_to = self.view.do_move(&self.text, movement,
            (flags & FLAG_SELECT) != 0);
    }

    fn move_up(&mut self, flags: u64) {
        self.do_move(Movement::Up, flags);
    }

    fn move_down(&mut self, flags: u64) {
        self.do_move(Movement::Down, flags);
    }

    fn move_left(&mut self, flags: u64) {
        self.do_move(Movement::Left, flags);
    }

    fn move_word_left(&mut self, flags: u64) {
        self.do_move(Movement::LeftWord, flags);
    }

    fn move_to_left_end_of_line(&mut self, flags: u64) {
        self.do_move(Movement::LeftOfLine, flags);
    }

    fn move_right(&mut self, flags: u64) {
        self.do_move(Movement::Right, flags);
    }

    fn move_word_right(&mut self, flags: u64) {
        self.do_move(Movement::RightWord, flags);
    }

    fn move_to_right_end_of_line(&mut self, flags: u64) {
        self.do_move(Movement::RightOfLine, flags);
    }

    fn move_to_beginning_of_paragraph(&mut self, flags: u64) {
        self.do_move(Movement::StartOfParagraph, flags);
    }

    fn move_to_end_of_paragraph(&mut self, flags: u64) {
        self.do_move(Movement::EndOfParagraph, flags);
    }

    fn move_to_beginning_of_document(&mut self, flags: u64) {
        self.do_move(Movement::StartOfDocument, flags);
    }

    fn move_to_end_of_document(&mut self, flags: u64) {
        self.do_move(Movement::EndOfDocument, flags);
    }

    fn scroll_page_up(&mut self, flags: u64) {
        self.do_move(Movement::UpPage, flags);
    }

    fn scroll_page_down(&mut self, flags: u64) {
        self.do_move(Movement::DownPage, flags);
    }

    fn select_all(&mut self) {
        self.view.select_all(self.text.len());
    }

    fn add_selection_by_movement(&mut self, movement: Movement) {
        let mut sel = Selection::new();
        for region in self.view.sel_regions() {
            sel.add_region(region.clone());
            let new_region = region_movement(movement, region, &self.view, &self.text, false);
            sel.add_region(new_region);
        }
        self.scroll_to = self.view.set_selection(&self.text, sel);
    }

    // TODO: insert from keyboard or input method shouldn't break undo group,
    // but paste should.
    fn do_insert(&mut self, chars: &str) {
        self.this_edit_type = EditType::InsertChars;
        self.insert(chars);
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

        self.pristine_rev_id = self.last_rev_id;
        self.view.set_pristine();
        self.view.set_dirty();
        self.render();
    }

    fn do_scroll(&mut self, first: i64, last: i64) {
        let first = max(first, 0) as usize;
        let last = last as usize;
        self.view.set_scroll(first, last);
        self.view.send_update_for_scroll(&self.text, &self.doc_ctx, &self.style_spans, first, last);
    }

    /// Sets the cursor and scrolls to the beginning of the given line.
    fn do_goto_line(&mut self, line: u64) {
        let line = self.view.line_col_to_offset(&self.text, line as usize, 0);
        self.set_cursor(line);
    }

    fn do_request_lines(&mut self, first: i64, last: i64) {
        self.view.send_update(&self.text, &self.doc_ctx, &self.style_spans, first as usize, last as usize);
    }

    fn do_click(&mut self, line: u64, col: u64, flags: u64, click_count: u64) {
        // TODO: calculate affinity
        let offset = self.view.line_col_to_offset(&self.text, line as usize, col as usize);
        if (flags & FLAG_SELECT) != 0 {
            if !self.view.is_point_in_selection(offset) {
                let sel = {
                    let (last, rest) = self.view.sel_regions().split_last().unwrap();
                    let mut sel = Selection::new();
                    for region in rest {
                        sel.add_region(region.clone());
                    }
                    // TODO: small nit, merged region should be backward if end < start.
                    // This could be done by explicitly overriding, or by tweaking the
                    // merge logic.
                    sel.add_region(SelRegion {
                        start: last.start,
                        end: offset,
                        horiz: None,
                        affinity: Affinity::default(),
                    });
                    sel
                };
                self.view.start_drag(offset, offset, offset);
                self.scroll_to = self.view.set_selection(&self.text, sel);
                return;
            }
        } else if click_count == 2 {
            let (start, end) = {
                let mut word_cursor = WordCursor::new(&self.text, offset);
                word_cursor.select_word()
            };
            self.set_sel_single_region(SelRegion{
                start: start,
                end: end,
                horiz: None,
                affinity: Affinity::default(),
            });
            self.view.start_drag(offset, start, end);
            return;
        } else if click_count == 3 {
            let start = self.view.line_col_to_offset(&self.text, line as usize, 0);
            let end = self.view.line_col_to_offset(&self.text, line as usize + 1, 0);
            self.set_sel_single_region(SelRegion{
                start: start,
                end: end,
                horiz: None,
                affinity: Affinity::default(),
            });
            self.view.start_drag(offset, start, end);
            return;
        }
        self.view.start_drag(offset, offset, offset);
        self.set_cursor(offset);
    }

    fn do_drag(&mut self, line: u64, col: u64, _flags: u64) {
        let offset = self.view.line_col_to_offset(&self.text, line as usize, col as usize);
        self.scroll_to = self.view.do_drag(&self.text, offset, Affinity::default());
    }

    fn do_gesture(&mut self, line: u64, col: u64, ty: GestureType) {
        let offset = self.view.line_col_to_offset(&self.text, line as usize, col as usize);
        match ty {
            GestureType::ToggleSel => self.view.toggle_sel(offset),
        }
    }

    fn debug_rewrap(&mut self) {
        self.view.rewrap(&self.text, 72);
        self.view.set_dirty();
    }

    fn debug_print_spans(&self) {
        // get last sel region
        let last_sel = self.view.sel_regions().last().unwrap();
        let iv = Interval::new_closed_open(last_sel.min(), last_sel.max());
        self.style_scopes.debug_print_spans(iv);
    }

    fn do_cut(&mut self) -> Value {
        let result = self.do_copy();
        // This copy is just to make the borrow checker happy, could be optimized.
        let deletions = self.view.sel_regions().to_vec();
        self.delete_sel_regions(&deletions);
        result
    }

    fn do_copy(&self) -> Value {
        if let Some(val) = self.extract_sel_regions(self.view.sel_regions()) {
            Value::String(val)
        } else {
            Value::Null
        }
    }

    fn do_undo(&mut self) {
        if self.cur_undo > 0 {
            self.cur_undo -= 1;
            assert!(self.undos.insert(self.live_undos[self.cur_undo]));
            self.this_edit_type = EditType::Undo;
            self.update_undos();
        }
    }

    fn do_redo(&mut self) {
        if self.cur_undo < self.live_undos.len() {
            assert!(self.undos.remove(&self.live_undos[self.cur_undo]));
            self.cur_undo += 1;
            self.this_edit_type = EditType::Redo;
            self.update_undos();
        }
    }

    fn do_transpose(&mut self) {
        let mut builder = delta::Builder::new(self.text.len());
        let mut last = 0;
        for region in self.view.sel_regions() {
            if region.is_caret() {
                let middle = region.end;
                let start = self.text.prev_grapheme_offset(middle).unwrap_or(0);
                // Note: this matches Sublime's behavior. Cocoa would swap last
                // two characters of line if at end of line.
                if let Some(end) = self.text.next_grapheme_offset(middle) {
                    if start >= last {
                        let interval = Interval::new_closed_open(start, end);
                        let swapped = self.text.slice_to_string(middle, end) +
                                      &self.text.slice_to_string(start, middle);
                        builder.replace(interval, Rope::from(swapped));
                        last = end;
                    }
                }
            }
            // TODO: handle else case by rotating non-caret regions.
        }
        if !builder.is_empty() {
            self.this_edit_type = EditType::Other;
            self.add_delta(builder.build());
        }
    }

    fn delete_to_end_of_paragraph(&mut self) {
        self.delete_by_movement(Movement::EndOfParagraphKill, true);
    }

    fn yank(&mut self) {
        // TODO: if there are multiple cursors and the number of newlines
        // is one less than the number of cursors, split and distribute one
        // line per cursor.
        let kill_ring_string = self.doc_ctx.get_kill_ring();
        self.insert(&*String::from(kill_ring_string));
    }

    pub fn do_find(&mut self, chars: Option<&str>, case_sensitive: bool) -> Option<Value> {
        let mut from_sel = false;
        let search_string = chars.map(String::from).or_else(|| {
            // if the search string is not provided, use the string of the last selection for find
            self.view.sel_regions().last().and_then(|region| {
                if region.is_caret() {
                    None
                } else {
                    from_sel = true;
                    Some(self.text.slice_to_string(region.min(), region.max()))
                }
            })
        });

        if search_string.is_none() {
            self.view.unset_find();
            return Some(Value::Null);
        }

        let search_string = search_string.unwrap();
        if search_string.len() == 0 {
            self.view.unset_find();
            return Some(Value::Null);
        }

        self.view.set_find(&search_string, case_sensitive);

        Some(Value::String(search_string.to_string()))
    }

    fn do_find_next(&mut self, reverse: bool, wrap_around: bool, allow_same: bool) {
        self.scroll_to = self.view.select_next_occurrence(&self.text, reverse, false, true, allow_same);

        if self.scroll_to.is_none() && wrap_around {
            // nothing found, search past end of file
            self.scroll_to = self.view.select_next_occurrence(&self.text, reverse, true, true, allow_same);
        }
    }

    pub fn do_rpc(&mut self, view_id: &ViewIdentifier, cmd: EditCommand) -> Option<Value> {
        use rpc::EditCommand::*;

        // if the rpc's originating view is different from current self.view, swap it in
        if self.view.view_id != *view_id {
            let mut temp = self.views.remove(view_id).expect("no view for provided view_id");
            mem::swap(&mut temp, &mut self.view);
            self.views.insert(temp.view_id.clone(), temp);
        }

        self.this_edit_type = EditType::Other;

        let result = match cmd {
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
            MoveWordLeft => async(self.move_word_left(0)),
            MoveWordLeftAndModifySelection => async(self.move_word_left(FLAG_SELECT)),
            MoveWordRight => async(self.move_word_right(0)),
            MoveWordRightAndModifySelection => async(self.move_word_right(FLAG_SELECT)),
            MoveToBeginningOfParagraph => async(self.move_to_beginning_of_paragraph(0)),
            MoveToEndOfParagraph => async(self.move_to_end_of_paragraph(0)),
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
            SelectAll => async(self.select_all()),
            AddSelectionAbove => async(self.add_selection_by_movement(Movement::Up)),
            AddSelectionBelow => async(self.add_selection_by_movement(Movement::Down)),
            Scroll { first, last } => async(self.do_scroll(first, last)),
            GotoLine { line } => async(self.do_goto_line(line)),
            RequestLines { first, last } => async(self.do_request_lines(first, last)),
            Yank => async(self.yank()),
            Transpose => async(self.do_transpose()),
            Click { line, column, flags, click_count } => {
                async(self.do_click(line, column, flags, click_count))
            }
            Drag { line, column, flags } => async(self.do_drag(line, column, flags)),
            Gesture { line, column, ty } => async(self.do_gesture(line, column, ty)),
            Undo => async(self.do_undo()),
            Redo => async(self.do_redo()),
            Cut => Some(self.do_cut()),
            Copy => Some(self.do_copy()),
            Find { chars, case_sensitive } => self.do_find(chars, case_sensitive),
            FindNext { wrap_around, allow_same } => async(self.do_find_next(false, wrap_around, allow_same)),
            FindPrevious { wrap_around } => async(self.do_find_next(true, wrap_around, true)),
            DebugRewrap => async(self.debug_rewrap()),
            DebugPrintSpans => async(self.debug_print_spans()),
        };

        // TODO: could defer this until input quiesces - will this help?
        self.commit_delta(None);
        self.render();
        self.last_edit_type = self.this_edit_type;
        result
    }

    // Note: the following are placeholders for prototyping, and are not intended to
    // deal with asynchrony or be efficient.

    pub fn plugin_n_lines(&self) -> usize {
        self.text.measure::<LinesMetric>() + 1
    }

    //TODO: plugins should optionally be able to provide a layer id
    // so a single plugin can maintain multiple layers
    pub fn plugin_add_scopes(&mut self, plugin: PluginPid, scopes: Vec<Vec<String>>) {
        self.style_scopes.add_scopes(plugin, scopes, &self.doc_ctx);
    }

    pub fn plugin_update_spans(&mut self, plugin: PluginPid, start: usize, len: usize,
                               spans: Vec<ScopeSpan>, rev: usize) {
        // TODO: more protection against invalid input
        let mut start = start;
        let mut end_offset = start + len;
        let mut sb = SpansBuilder::new(len);
        for span in spans {
            sb.add_span(Interval::new_open_open(span.start, span.end), span.scope_id);
        }
        let mut spans = sb.build();
        if rev != self.engine.get_head_rev_id() {
            let delta = self.engine.delta_rev_head(rev);
            let mut transformer = Transformer::new(&delta);
            let new_start = transformer.transform(start, false);
            if !transformer.interval_untouched(
                Interval::new_closed_closed(start, end_offset)) {
                spans = spans.transform(start, end_offset, &mut transformer);
            }
            start = new_start;
            end_offset = transformer.transform(end_offset, true);
        }
        let iv = Interval::new_closed_closed(start, end_offset);
        self.style_scopes.update_layer(plugin, iv, spans);
        let updated_styles = self.style_scopes.resolve_styles(iv);
        self.style_spans.edit(iv, updated_styles);
        self.view.set_dirty();
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

    // Note: currently we route up through Editor to DocumentCtx, but perhaps the plugin
    // should have its own reference.
    pub fn plugin_alert(&self, msg: &str) {
        self.doc_ctx.alert(msg);
    }

    /// Notifies the client that the named plugin has started.
    ///
    /// Note: there is no current conception of a plugin which is only active
    /// for a particular view; plugins are active at the editor/buffer level.
    /// Some `view_id` is needed, however, to route to the correct client view.
    //TODO: revisit this after implementing multiview
    pub fn plugin_started<'a, T>(&'a self, view_id: T, plugin: &str)
        where T: Into<Option<&'a ViewIdentifier>> {
        let view_id = view_id.into().unwrap_or(&self.view.view_id);
        self.doc_ctx.plugin_started(view_id, plugin);
    }

    /// Notifies client that the named plugin has stopped.
    ///
    /// `code` is reserved for future use.
    pub fn plugin_stopped<'a, T>(&'a mut self, view_id: T, plugin: &str,
                                 plugin_id: PluginPid, code: i32)
        where T: Into<Option<&'a ViewIdentifier>> {
        {
            self.style_scopes.remove_layer(plugin_id);
            let iv_all = Interval::new_open_closed(0, self.text.len());
            self.style_spans = self.style_scopes.resolve_styles(iv_all);
            self.view.set_dirty();
            self.render();
        }
        let view_id = view_id.into().unwrap_or(&self.view.view_id);
        self.doc_ctx.plugin_stopped(view_id, plugin, code);
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
