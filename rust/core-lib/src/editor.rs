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

use std::borrow::{Borrow, Cow};
use std::cmp::{min, max};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::io::Write;
use std::collections::BTreeSet;
use std::time::SystemTime;

use serde_json::Value;

use xi_rope::rope::{LinesMetric, Rope, RopeInfo};
use xi_rope::interval::Interval;
use xi_rope::delta::{self, Delta, Transformer};
use xi_rope::engine::{Engine, RevId, RevToken};
use xi_rope::spans::SpansBuilder;
use xi_rpc::RemoteError;
use xi_trace::trace_block;

use view::View;
use word_boundaries::WordCursor;
use movement::{Movement, region_movement};
use selection::{Affinity, Selection, SelRegion};

use tabs::{self, BufferIdentifier, ViewIdentifier, DocumentCtx};
use rpc::{self, GestureType};
use syntax::SyntaxDefinition;
use plugins::rpc::{PluginUpdate, PluginEdit, ScopeSpan, PluginBufferInfo,
ClientPluginInfo, TextUnit, GetDataResponse};
use plugins::{PluginPid, Command};
use layers::Scopes;
use config::{BufferConfig, Table};


#[cfg(not(feature = "ledger"))]
pub struct SyncStore;
#[cfg(feature = "ledger")]
use fuchsia::sync::SyncStore;

const FLAG_SELECT: u64 = 2;

// TODO This could go much higher without issue but while developing it is
// better to keep it low to expose bugs in the GC during casual testing.
const MAX_UNDOS: usize = 20;

// Maximum returned result from plugin get_data RPC.
const MAX_SIZE_LIMIT: usize = 1024 * 1024;

enum CharacterEncoding {
    Utf8,
    Utf8WithBom
}

const UTF8_BOM: &str = "\u{feff}";

fn last_selection_region(regions: &[SelRegion]) -> Option<&SelRegion> {
    for region in regions.iter().rev() {
        if !region.is_caret() {
            return Some(region);
        }
    }

    None
}

pub struct Editor {
    text: Rope,
    encoding: CharacterEncoding,

    path: Option<PathBuf>,
    file_mod_time: Option<SystemTime>,
    file_has_changed: bool,
    buffer_id: BufferIdentifier,
    syntax: SyntaxDefinition,
    view: View,
    engine: Engine,
    last_rev_id: RevId,
    pristine_rev_id: RevId,
    undo_group_id: usize,
    live_undos: Vec<usize>, //  undo groups that may still be toggled
    cur_undo: usize, // index to live_undos, ones after this are undone
    undos: BTreeSet<usize>, // undo groups that are undone
    gc_undos: BTreeSet<usize>, // undo groups that are no longer live and should be gc'ed

    this_edit_type: EditType,
    last_edit_type: EditType,

    scroll_to: Option<usize>,

    styles: Scopes,
    doc_ctx: DocumentCtx,
    config: BufferConfig,
    revs_in_flight: usize,

    /// Used only on Fuchsia for syncing
    #[allow(dead_code)]
    sync_store: Option<SyncStore>,
    #[allow(dead_code)]
    last_synced_rev: RevId,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum EditType {
    Other,
    InsertChars,
    Delete,
    Undo,
    Redo,
    Transpose,
}

impl EditType {
    pub fn json_string(&self) -> &'static str {
        match *self {
            EditType::InsertChars => "insert",
            EditType::Delete => "delete",
            EditType::Undo => "undo",
            EditType::Redo => "redo",
            EditType::Transpose => "transpose",
            _ => "other",
        }
    }
}

impl Editor {
    /// Creates a new `Editor` with a new empty buffer.
    pub fn new(doc_ctx: DocumentCtx, config: BufferConfig,
               buffer_id: BufferIdentifier,
               initial_view_id: ViewIdentifier) -> Editor {
        Self::with_text(doc_ctx, config, buffer_id,
                        initial_view_id, "".to_owned())
    }

    /// Creates a new `Editor`, loading text into a new buffer.
    pub fn with_text(doc_ctx: DocumentCtx, config: BufferConfig,
                     buffer_id: BufferIdentifier,
                     initial_view_id: ViewIdentifier, text: String) -> Editor {

        let encoding = if text.starts_with(UTF8_BOM) {
            CharacterEncoding::Utf8WithBom
        } else {
            CharacterEncoding::Utf8
        };

        let engine = Engine::new(Rope::from(match encoding {
            CharacterEncoding::Utf8WithBom => &text[UTF8_BOM.len()..],
            CharacterEncoding::Utf8 => text.as_str()
        }));
        let buffer = engine.get_head().clone();
        let last_rev_id = engine.get_head_rev_id();

        let mut editor = Editor {
            text: buffer,
            encoding: encoding,
            buffer_id: buffer_id,
            path: None,
            file_mod_time: None,
            file_has_changed: false,
            syntax: SyntaxDefinition::default(),
            view: View::new(initial_view_id),
            engine: engine,
            last_rev_id: last_rev_id,
            pristine_rev_id: last_rev_id,
            undo_group_id: 1,
            // GC only works on undone edits or prefixes of the visible edits,
            // but initial file loading can create an edit with undo group 0,
            // so we want to collect that as part of the prefix.
            live_undos: vec![0],
            cur_undo: 1,
            undos: BTreeSet::new(),
            gc_undos: BTreeSet::new(),
            last_edit_type: EditType::Other,
            this_edit_type: EditType::Other,
            scroll_to: Some(0),
            styles: Scopes::default(),
            doc_ctx: doc_ctx,
            config: config,
            revs_in_flight: 0,
            sync_store: None,
            last_synced_rev: last_rev_id,
        };
        editor.view.rewrap(&editor.text, editor.config.items.wrap_width);
        editor.view.set_dirty(&editor.text);
        editor
    }

    /// should only ever be called from `BufferContainerRef::set_path`
    #[doc(hidden)]
    pub fn _set_path<P: AsRef<Path>>(&mut self, path: P) {
        let path = path.as_ref();
        //TODO: if the user sets syntax, we shouldn't overwrite here
        self.syntax = SyntaxDefinition::new(path.to_str());
        self.file_mod_time = tabs::get_file_mod_time(path);
        self.path = Some(path.to_owned());
    }

    /// If this `Editor`'s buffer has been saved, Returns its path.
    pub fn get_path(&self) -> Option<&Path> {
        match self.path {
            Some(ref p) => Some(p),
            None => None,
        }
    }

    /// Returns the time of the last file write initiated by this `Editor`.
    pub fn get_file_mod_time(&self) -> Option<SystemTime> {
        self.file_mod_time
    }

    /// Returns `true` if this editor's file has changed on disk.
    pub fn get_file_has_changed(&self) -> bool {
        self.file_has_changed
    }

    #[doc(hidden)]
    pub (crate) fn _set_file_has_changed(&mut self, has_changed: bool) {
        self.file_has_changed = has_changed
    }

    /// Sets this Editor's contents to `text`, preserving undo state and cursor
    /// position when possible.
    pub fn reload(&mut self, text: &str) {
        self.this_edit_type = EditType::Other;
        let new_text = Rope::from(text);
        let new_len = new_text.len();

        // preserve a single caret
        self.view.collapse_selections(&self.text);
        let prev_sel = self.view.sel_regions().first().map(|s| s.clone());
        self.view.unset_find(&self.text);

        let mut builder = delta::Builder::new(self.text.len());
        let all_iv = Interval::new_closed_open(0, self.text.len());
        builder.replace(all_iv, new_text);
        self.add_delta(builder.build());
        self.commit_delta(None);
        self.last_edit_type = EditType::Other;

        if let Some(prev_sel) = prev_sel {
            let offset = prev_sel.start.min(new_len);
            self.set_selection(SelRegion::caret(offset));
        }

        self.file_mod_time = self.path.as_ref()
            .and_then(tabs::get_file_mod_time);
        self.pristine_rev_id = self.last_rev_id;
        self.view.set_pristine();
        self.render()
    }

    /// Sets the config for this buffer. If the new config differs
    /// from the existing config, returns the modified items.
    pub fn set_config(&mut self, conf: BufferConfig) -> Option<Table> {
        if let Some(changes) = conf.changes_from(Some(&self.config)) {
            self.config = conf;
            if changes.contains_key("wrap_width") {
                self.view.rewrap(&self.text, self.config.items.wrap_width);;
                self.view.set_dirty(&self.text);
                self.render();
            }
            self.doc_ctx.config_changed(&self.view.view_id, &changes);
            Some(changes)
        } else {
            None
        }
    }

    pub fn get_config(&self) -> &BufferConfig {
        &self.config
    }

    /// Returns this `Editor`'s active `SyntaxDefinition`.
    pub fn get_syntax(&self) -> &SyntaxDefinition {
        &self.syntax
    }

    /// Returns this `Editor`'s `BufferIdentifier`.
    pub fn get_identifier(&self) -> BufferIdentifier {
        self.buffer_id
    }

    /// returns the `ViewIdentifier` of the current view.
    pub fn get_main_view_id(&self) -> ViewIdentifier {
        self.view.view_id
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
        let views = vec![self.view.view_id];
        let config = self.config.to_table();
        PluginBufferInfo::new(self.buffer_id, &views,
                              self.engine.get_head_rev_id().token(), self.text.len(),
                              nb_lines, self.path.clone(), self.syntax.clone(),
                              config)
    }

    /// Send initial config state to the client.
    pub fn send_config_init(&self) {
        let config = self.config.to_table();
        self.doc_ctx.config_changed(&self.view.view_id, &config);
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

    /// Sets the selection and scrolls the end of it into view.
    fn set_selection<S: Into<Selection>>(&mut self, sel: S) {
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
            self.this_edit_type != EditType::Other && self.this_edit_type != EditType::Transpose &&
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
        self.engine.edit_rev(priority, undo_group, head_rev_id.token(), delta);
        self.text = self.engine.get_head().clone();
    }

    /// Commits the current delta, updating views, plugins, and other invariants as needed.
    fn commit_delta(&mut self, author: Option<&str>) {
        if self.engine.get_head_rev_id() != self.last_rev_id {
            self.update_after_revision(author);
        }
    }

    /// generates a delta from a plugin's response and applies it to the buffer.
    pub fn apply_plugin_edit(&mut self, edit: PluginEdit, undo_group: Option<usize>) {
        if let Some(undo_group) = undo_group {
            // non-async edits modify their associated revision
            //TODO: get priority working, so that plugin edits don't necessarily move cursor
            self.engine.edit_rev(edit.priority as usize, undo_group, edit.rev, edit.delta);
            self.text = self.engine.get_head().clone();
        }
        else {
            self.add_delta(edit.delta);
        }

        self.commit_delta(Some(&edit.author));
        self.render();
    }

    fn update_undos(&mut self) {
        self.engine.undo(self.undos.clone());
        self.text = self.engine.get_head().clone();
        self.update_after_revision(None);
    }

    fn update_after_revision(&mut self, author: Option<&str>) {
        let _t = trace_block("Editor::update_after_rev", &["core"]);
        let last_token = self.last_rev_id.token();
        let delta = self.engine.delta_rev_head(last_token);
        let is_pristine = self.is_pristine();
        // TODO (performance): it's probably quicker to stash last_text rather than
        // resynthesize it.
        let last_text = self.engine.get_rev(last_token).expect("last_rev not found");
        let keep_selections = self.this_edit_type == EditType::Transpose;
        self.scroll_to = self.view.after_edit(&self.text, &last_text, &delta, is_pristine, keep_selections);
        let (iv, new_len) = delta.summary();
        let total_num_lines = self.text.measure::<LinesMetric>() + 1;

        // TODO: perhaps use different semantics for spans that enclose the
        // edited region. Currently it breaks any such span in half and applies
        // no spans to the inserted text. That's ok for syntax highlighting but
        // not ideal for rich text.
        self.styles.update_all(iv, new_len);

        // We increment revs in flight once here, and we decrement once
        // after sending plugin updates, regardless of whether or not any actual
        // plugins get updated. This ensures that gc runs.
        self.increment_revs_in_flight();

        {
            let new_len = delta.new_document_len();
            let approx_delta_size = delta.inserts_len() + (delta.els.len() * 10);
            let delta = match approx_delta_size > MAX_SIZE_LIMIT {
                true => None,
                false => Some(delta),
            };
            let author = match author {
                Some(s) => s.to_owned(),
                None => self.view.view_id.to_string(),
            };

            let update = PluginUpdate::new(
                self.view.view_id,
                self.engine.get_head_rev_id().token(),
                delta,
                new_len,
                total_num_lines,
                self.this_edit_type.json_string().to_owned(),
                author.to_owned());

            let undo_group = *self.live_undos.last().unwrap_or(&0);
            let view_id = self.view.view_id;
            self.doc_ctx.update_plugins(view_id, update, undo_group);
        }


        self.last_rev_id = self.engine.get_head_rev_id();
        self.sync_state_changed();
    }

    #[cfg(not(target_os = "fuchsia"))]
    fn gc_undos(&mut self) {
        if self.revs_in_flight == 0 && !self.gc_undos.is_empty() {
            self.engine.gc(&self.gc_undos);
            self.undos = &self.undos - &self.gc_undos;
            self.gc_undos.clear();
        }
    }

    #[cfg(target_os = "fuchsia")]
    fn gc_undos(&mut self) {
        // Never run GC on Fuchsia so that peers don't invalidate our
        // last_rev_id and so that merge will work.
    }

    pub (crate) fn is_pristine(&self) -> bool {
        self.engine.is_equivalent_revision(self.pristine_rev_id, self.engine.get_head_rev_id())
    }

    // render if needed, sending to ui
    pub fn render(&mut self) {
        let _t = trace_block("Editor::render", &["core"]);
        self.view.render_if_dirty(&self.text, &self.doc_ctx, self.styles.get_merged());
        if let Some(scrollto) = self.scroll_to {
            let (line, col) = self.view.offset_to_line_col(&self.text, scrollto);
            self.doc_ctx.scroll_to(self.view.view_id, line, col);
            self.scroll_to = None;
        }
    }

    pub fn merge_new_state(&mut self, new_engine: Engine) {
        self.engine.merge(&new_engine);
        self.text = self.engine.get_head().clone();
        // TODO: better undo semantics. This only implements separate undo histories for low concurrency.
        self.undo_group_id = self.engine.max_undo_group_id() + 1;
        self.last_synced_rev = self.engine.get_head_rev_id();
        self.commit_delta(None);
        self.render();
    }

    /// See `Engine::set_session_id` only useful when using Fuchsia sync functionality.
    pub fn set_session_id(&mut self, session: (u64,u32)) {
        self.engine.set_session_id(session);
    }

    #[cfg(feature = "ledger")]
    pub fn set_sync_store(&mut self, sync_store: SyncStore) {
        self.sync_store = Some(sync_store);
    }

    #[cfg(not(feature = "ledger"))]
    pub fn sync_state_changed(&mut self) {
    }

    #[cfg(feature = "ledger")]
    pub fn sync_state_changed(&mut self) {
        if let Some(sync_store) = self.sync_store.as_mut() {
            // we don't want to sync right after recieving a new merge
            if self.last_synced_rev != self.engine.get_head_rev_id() {
                self.last_synced_rev = self.engine.get_head_rev_id();
                sync_store.state_changed();
            }
        }
    }

    #[cfg(feature = "ledger")]
    pub fn transaction_ready(&mut self) {
        if let Some(sync_store) = self.sync_store.as_mut() {
            sync_store.commit_transaction(&self.engine);
        }
    }

    fn delete_word_forward(&mut self) {
        self.delete_by_movement(Movement::RightWord, false);
    }

    fn delete_word_backward(&mut self) {
        self.delete_by_movement(Movement::LeftWord, false);
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
                // backspace deletes max(1, tab_size) contiguous spaces
                let (_, c) = self.view.offset_to_line_col(&self.text,
                                                          region.start);
                let use_spaces = self.config.items.translate_tabs_to_spaces;
                let use_tab_stops = self.config.items.use_tab_stops;
                let tab_size = self.config.items.tab_size;
                let tab_size = if c % tab_size == 0 { tab_size } else { c % tab_size };
                let preceded_by_spaces = self.text.len() > 0 &&
                    (region.start.saturating_sub(tab_size)..region.start)
                    .all(|i| self.text.byte_at(i) == b' ');
               if preceded_by_spaces && use_spaces && use_tab_stops {
                   region.start - tab_size
               } else {
                   // TODO: implement complex emoji logic
                    self.text.prev_codepoint_offset(region.end)
                        .unwrap_or(region.end)
               }
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
        for &r in self.view.sel_regions() {
            if r.is_caret() {
                let new_region = region_movement(movement, r, &self.view, &self.text, true);
                deletions.add_region(new_region);
            } else {
                deletions.add_region(r);
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
        let text = self.config.items.line_ending.clone();
        self.insert(&text);
    }

    fn insert_tab(&mut self) {
        let mut builder = delta::Builder::new(self.text.len());
        for region in self.view.sel_regions() {
            let iv = Interval::new_closed_open(region.min(), region.max());
            let tab_text = if self.config.items.translate_tabs_to_spaces {
                    let (_, col) = self.view.offset_to_line_col(&self.text, region.start);
                    let tab_size = self.config.items.tab_size;
                    let n = tab_size - (col % tab_size);
                    n_spaces(n)
            } else {
                "\t"
            };
            builder.replace(iv, Rope::from(tab_text));
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
        self.view.select_all(&self.text);
    }

    fn add_selection_by_movement(&mut self, movement: Movement) {
        let mut sel = Selection::new();
        for &region in self.view.sel_regions() {
            sel.add_region(region);
            let new_region = region_movement(movement, region, &self.view, &self.text, false);
            sel.add_region(new_region);
        }
        self.set_selection(sel);
    }

    // TODO: insert from keyboard or input method shouldn't break undo group,
    // but paste should.
    fn do_insert(&mut self, chars: &str) {
        self.this_edit_type = EditType::InsertChars;
        self.insert(chars);
    }

    pub fn do_save<P: AsRef<Path>>(&mut self, path: P) -> Result<(), String> {
        match File::create(&path) {
            Ok(mut f) => {
                if let Err(e) = match self.encoding {
                    CharacterEncoding::Utf8WithBom => f.write_all(UTF8_BOM.as_bytes()),
                    CharacterEncoding::Utf8 => Result::Ok(())
                } {
                    Err(format!("write error {}", e))
                } else {
                    for chunk in self.text.iter_chunks(0, self.text.len()) {
                        if let Err(e) = f.write_all(chunk.as_bytes()) {
                            return Err(format!("write error {}", e));
                        }
                    }
                    self.pristine_rev_id = self.last_rev_id;
                    self.view.set_pristine();
                    self.view.set_dirty(&self.text);
                    self.render();
                    Ok(())
                }
            }
            Err(e) => Err(format!("create error {}", e)),
        }
    }

    fn do_scroll(&mut self, first: i64, last: i64) {
        let first = max(first, 0) as usize;
        let last = max(last, 0) as usize;
        self.view.set_scroll(first, last);
    }

    /// Sets the cursor and scrolls to the beginning of the given line.
    fn do_goto_line(&mut self, line: u64) {
        let offset = self.view.line_col_to_offset(&self.text, line as usize, 0);
        self.set_selection(SelRegion::caret(offset));
    }

    fn do_request_lines(&mut self, first: i64, last: i64) {
        self.view.request_lines(&self.text, &self.doc_ctx, self.styles.get_merged(), first as usize, last as usize);
    }

    fn do_click(&mut self, line: u64, col: u64, flags: u64, click_count: u64) {
        // TODO: calculate affinity
        let offset = self.view.line_col_to_offset(&self.text, line as usize, col as usize);
        if (flags & FLAG_SELECT) != 0 {
            if !self.view.is_point_in_selection(offset) {
                let sel = {
                    let (last, rest) = self.view.sel_regions().split_last().unwrap();
                    let mut sel = Selection::new();
                    for &region in rest {
                        sel.add_region(region);
                    }
                    // TODO: small nit, merged region should be backward if end < start.
                    // This could be done by explicitly overriding, or by tweaking the
                    // merge logic.
                    sel.add_region(SelRegion::new(last.start, offset));
                    sel
                };
                self.view.start_drag(offset, offset, offset);
                self.set_selection(sel);
                return;
            }
        } else if click_count == 2 {
            self.view.select_word(&self.text, offset, false);
            return;
        } else if click_count == 3 {
            self.view.select_line(&self.text, offset, line as usize, false);
            return;
        }
        self.view.start_drag(offset, offset, offset);
        self.set_selection(SelRegion::caret(offset));
    }

    fn do_drag(&mut self, line: u64, col: u64, _flags: u64) {
        let offset = self.view.line_col_to_offset(&self.text, line as usize, col as usize);
        self.scroll_to = self.view.do_drag(&self.text, offset, Affinity::default());
    }

    fn do_gesture(&mut self, line: u64, col: u64, ty: GestureType) {
        let offset = self.view.line_col_to_offset(&self.text, line as usize, col as usize);
        match ty {
            GestureType::ToggleSel => self.view.toggle_sel(&self.text, offset),
            GestureType::MultiLineSelect => self.view.select_line(&self.text, offset, line as usize, true),
            GestureType::MultiWordSelect => self.view.select_word(&self.text, offset, true)
        }
    }

    fn debug_rewrap(&mut self) {
        self.view.rewrap(&self.text, 72);
        self.view.set_dirty(&self.text);
    }

    fn debug_print_spans(&self) {
        // get last sel region
        let last_sel = self.view.sel_regions().last().unwrap();
        let iv = Interval::new_closed_open(last_sel.min(), last_sel.max());
        self.styles.debug_print_spans(iv);
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
        if self.cur_undo > 1 {
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

    fn sel_region_to_interval_and_rope(&self, region: SelRegion) -> (Interval, Rope) {
        let as_interval = Interval::new_closed_open(region.min(), region.max());
        let interval_rope = Rope::from(self.text.slice_to_string(
            as_interval.start(), as_interval.end()));
        (as_interval, interval_rope)
    }

    fn do_transpose(&mut self) {
        let mut builder = delta::Builder::new(self.text.len());
        let mut last = 0;
        let mut optional_previous_selection : Option<(Interval, Rope)> =
            last_selection_region(self.view.sel_regions()).map(
                |&region| self.sel_region_to_interval_and_rope(region));

        for &region in self.view.sel_regions() {
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
            } else if let Some(previous_selection) = optional_previous_selection {
                let current_interval = self.sel_region_to_interval_and_rope(region);
                builder.replace(current_interval.0, previous_selection.1);
                optional_previous_selection = Some(current_interval);
            }
        }
        if !builder.is_empty() {
            self.this_edit_type = EditType::Transpose;
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

    pub fn do_find(&mut self, chars: Option<String>, case_sensitive: bool) -> Value {
        let mut from_sel = false;
        let search_string = if chars.is_some() {
            chars
        } else {
            self.view.sel_regions().last().and_then(|region| {
                if region.is_caret() {
                    None
                } else {
                    from_sel = true;
                    Some(self.text.slice_to_string(region.min(), region.max()))
                }
            })
        };

        if search_string.is_none() {
            self.view.unset_find(&self.text);
            return Value::Null;
        }

        let search_string = search_string.unwrap();
        if search_string.len() == 0 {
            self.view.unset_find(&self.text);
            return Value::Null;
        }

        self.view.set_find(&self.text, &search_string, case_sensitive);

        Value::String(search_string.to_string())
    }

    fn do_find_next(&mut self, reverse: bool, wrap_around: bool, allow_same: bool) {
        self.scroll_to = self.view.select_next_occurrence(&self.text, reverse, false, true, allow_same);

        if self.scroll_to.is_none() && wrap_around {
            // nothing found, search past end of file
            self.scroll_to = self.view.select_next_occurrence(&self.text, reverse, true, true, allow_same);
        }
    }

    fn do_cancel_operation(&mut self) {
        self.view.unset_find(&self.text);
        self.view.collapse_selections(&self.text);
    }

    fn transform_text<F: Fn(&str) -> String>(&mut self, transform_function: F) {
        let mut builder = delta::Builder::new(self.text.len());

        for region in self.view.sel_regions() {
            let selected_text = self.text.slice_to_string(region.min(), region.max());
            let interval = Interval::new_closed_open(region.min(), region.max());
            builder.replace(interval, Rope::from(transform_function(&selected_text)));
        }
        if !builder.is_empty() {
            self.this_edit_type = EditType::Other;
            self.add_delta(builder.build());
        }
    }

    fn cmd_prelude(&mut self) {
        self.this_edit_type = EditType::Other;
    }

    fn cmd_postlude(&mut self) {
        // TODO: could defer this until input quiesces - will this help?
        self.commit_delta(None);
        self.render();
        self.last_edit_type = self.this_edit_type;
    }

    pub fn handle_notification(&mut self, _view_id: ViewIdentifier,
                               cmd: rpc::EditNotification) {

        let _t = trace_block("Editor::handle_notif", &["core"]);

        use rpc::EditNotification::*;
        use rpc::{LineRange, MouseAction};
        self.cmd_prelude();

        match cmd {
            Insert { chars } => self.do_insert(&chars),
            DeleteForward => self.delete_forward(),
            DeleteBackward => self.delete_backward(),
            DeleteWordForward => self.delete_word_forward(),
            DeleteWordBackward => self.delete_word_backward(),
            DeleteToEndOfParagraph => self.delete_to_end_of_paragraph(),
            DeleteToBeginningOfLine => self.delete_to_beginning_of_line(),
            InsertNewline => self.insert_newline(),
            InsertTab => self.insert_tab(),
            MoveUp => self.move_up(0),
            MoveUpAndModifySelection => self.move_up(FLAG_SELECT),
            MoveDown => self.move_down(0),
            MoveDownAndModifySelection => self.move_down(FLAG_SELECT),
            MoveLeft | MoveBackward => self.move_left(0),
            MoveLeftAndModifySelection => self.move_left(FLAG_SELECT),
            MoveRight | MoveForward => self.move_right(0),
            MoveRightAndModifySelection => self.move_right(FLAG_SELECT),
            MoveWordLeft => self.move_word_left(0),
            MoveWordLeftAndModifySelection => self.move_word_left(FLAG_SELECT),
            MoveWordRight => self.move_word_right(0),
            MoveWordRightAndModifySelection => self.move_word_right(FLAG_SELECT),
            MoveToBeginningOfParagraph => self.move_to_beginning_of_paragraph(0),
            MoveToEndOfParagraph => self.move_to_end_of_paragraph(0),
            MoveToLeftEndOfLine => self.move_to_left_end_of_line(0),
            MoveToLeftEndOfLineAndModifySelection => self.move_to_left_end_of_line(FLAG_SELECT),
            MoveToRightEndOfLine => self.move_to_right_end_of_line(0),
            MoveToRightEndOfLineAndModifySelection => self.move_to_right_end_of_line(FLAG_SELECT),
            MoveToBeginningOfDocument => self.move_to_beginning_of_document(0),
            MoveToBeginningOfDocumentAndModifySelection => self.move_to_beginning_of_document(FLAG_SELECT),
            MoveToEndOfDocument => self.move_to_end_of_document(0),
            MoveToEndOfDocumentAndModifySelection => self.move_to_end_of_document(FLAG_SELECT),
            ScrollPageUp => self.scroll_page_up(0),
            PageUpAndModifySelection => self.scroll_page_up(FLAG_SELECT),
            ScrollPageDown => self.scroll_page_down(0),
            PageDownAndModifySelection => {
                self.scroll_page_down(FLAG_SELECT)
            }
            SelectAll => self.select_all(),
            AddSelectionAbove => self.add_selection_by_movement(Movement::Up),
            AddSelectionBelow => self.add_selection_by_movement(Movement::Down),
            Scroll(LineRange { first, last }) => self.do_scroll(first, last),
            GotoLine { line } => self.do_goto_line(line),
            RequestLines(LineRange { first, last }) => self.do_request_lines(first, last),
            Yank => self.yank(),
            Transpose => self.do_transpose(),
            Click(MouseAction {line, column, flags, click_count} ) => {
                self.do_click(line, column, flags, click_count.unwrap())
            }
            Drag (MouseAction {line, column, flags, ..}) => {
                self.do_drag(line, column, flags);
            }
            Gesture { line, col, ty } => self.do_gesture(line, col, ty),
            Undo => self.do_undo(),
            Redo => self.do_redo(),
            FindNext { wrap_around, allow_same } => self.do_find_next(false, wrap_around.unwrap_or(false), allow_same.unwrap_or(false)),
            FindPrevious { wrap_around } => self.do_find_next(true, wrap_around.unwrap_or(false), true),
            DebugRewrap => self.debug_rewrap(),
            DebugPrintSpans => self.debug_print_spans(),
            CancelOperation => self.do_cancel_operation(),
            Uppercase => self.transform_text(|s| s.to_uppercase()),
            Lowercase => self.transform_text(|s| s.to_lowercase()),
        };

        self.cmd_postlude();
    }


    pub fn handle_request(&mut self, _view_id: ViewIdentifier,
                          cmd: rpc::EditRequest) -> Result<Value, RemoteError> {
        use rpc::EditRequest::*;
        let _t = trace_block("Editor::handle_request", &["core"]);
        self.cmd_prelude();

        let result = match cmd {
            Cut => self.do_cut(),
            Copy => self.do_copy(),
            Find { chars, case_sensitive } => self.do_find(chars, case_sensitive),
        };

        self.cmd_postlude();
        Ok(result)
    }

    pub fn theme_changed(&mut self) {
        self.styles.theme_changed(&self.doc_ctx);
        self.view.set_dirty(&self.text);
        self.render();
    }

    // Note: the following are placeholders for prototyping, and are not intended to
    // deal with asynchrony or be efficient.

    /// Applies an async edit from a plugin.
    pub fn plugin_edit_async(&mut self, edit: PluginEdit) {
        let _t = trace_block("Editor::plugin_edit", &["core"]);
        self.this_edit_type = EditType::Other;
        self.apply_plugin_edit(edit, None)
    }

    pub fn plugin_n_lines(&self) -> usize {
        self.text.measure::<LinesMetric>() + 1
    }

    //TODO: plugins should optionally be able to provide a layer id
    // so a single plugin can maintain multiple layers
    pub fn plugin_add_scopes(&mut self, plugin: PluginPid, scopes: Vec<Vec<String>>) {
        let _t = trace_block("Editor::add_scopes", &["core"]);
        self.styles.add_scopes(plugin, scopes, &self.doc_ctx);
    }

    pub fn plugin_update_spans(&mut self, plugin: PluginPid, start: usize, len: usize,
                               spans: Vec<ScopeSpan>, rev: RevToken) {
        let _t = trace_block("Editor::update_spans", &["core"]);
        // TODO: more protection against invalid input
        let mut start = start;
        let mut end_offset = start + len;
        let mut sb = SpansBuilder::new(len);
        for span in spans {
            sb.add_span(Interval::new_open_open(span.start, span.end), span.scope_id);
        }
        let mut spans = sb.build();
        if rev != self.engine.get_head_rev_id().token() {
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
        self.styles.update_layer(plugin, iv, spans);
        self.view.invalidate_styles(&self.text, start, end_offset);
        self.render();
    }

    pub fn plugin_get_data(&self, start: usize, unit: TextUnit,
                           max_size: usize, rev: RevToken) -> Option<GetDataResponse> {
        let _t = trace_block("Editor::plugin_get_data", &["core"]);
        let text_cow = if rev == self.engine.get_head_rev_id().token() {
            Cow::Borrowed(&self.text)
        } else {
            match self.engine.get_rev(rev) {
                None => return None,
                Some(text) => Cow::Owned(text)
            }
        };
        let text = &text_cow;
        // convert our offset into a valid byte offset
        let offset = unit.resolve_offset(text.borrow(), start)?;

        let max_size = min(max_size, MAX_SIZE_LIMIT);
        let mut end_off = offset.saturating_add(max_size);
        if end_off >= text.len() {
            end_off = text.len();
        } else {
            // Snap end to codepoint boundary.
            end_off = text.prev_codepoint_offset(end_off + 1).unwrap();
        }

        let chunk = text.slice_to_string(offset, end_off);
        let first_line = text.line_of_offset(offset);
        let first_line_offset = offset - text.offset_of_line(first_line);

        Some(GetDataResponse { chunk, offset, first_line, first_line_offset })
    }

    pub fn plugin_get_selections(&self, view_id: ViewIdentifier) -> Value {
        //TODO: multiview support
        assert_eq!(view_id, self.view.view_id);
        let sels: Vec<(usize, usize)> = self.view.sel_regions()
            .iter()
            .map(|s| { (s.start, s.end) })
            .collect();

        json!({"selections": sels})
    }

    // Note: currently we route up through Editor to DocumentCtx, but perhaps the plugin
    // should have its own reference.
    pub fn plugin_alert(&self, msg: &str) {
        self.doc_ctx.alert(msg);
    }

    /// Notifies the client of the currently available plugins.
    pub fn available_plugins(&self, view_id: ViewIdentifier,
                             plugins: &[ClientPluginInfo]) {
        self.doc_ctx.available_plugins(view_id, plugins);
    }

    /// Notifies the client that the named plugin has started.
    ///
    /// Note: there is no current conception of a plugin which is only active
    /// for a particular view; plugins are active at the editor/buffer level.
    /// Some `view_id` is needed, however, to route to the correct client view.
    //TODO: revisit this after implementing multiview
    pub fn plugin_started<T>(&self, view_id: T, plugin: &str, cmds: &[Command])
        where T: Into<Option<ViewIdentifier>>
    {
        let _t = trace_block("Editor::plugin_started", &["core"]);
        let view_id = view_id.into().unwrap_or(self.view.view_id);
        self.doc_ctx.plugin_started(view_id, plugin);
        self.doc_ctx.update_cmds(view_id, plugin, cmds);
    }

    /// Notifies client that the named plugin has stopped.
    ///
    /// `code` is reserved for future use.
    pub fn plugin_stopped<'a, T>(&'a mut self, view_id: T, plugin: &str,
                                 plugin_id: PluginPid, code: i32)
        where T: Into<Option<ViewIdentifier>> {
        let _t = trace_block("Editor::plugin_stopped", &["core"]);
        {
            self.styles.remove_layer(plugin_id);
            self.view.set_dirty(&self.text);
            self.render();
        }
        let view_id = view_id.into().unwrap_or(self.view.view_id);
        self.doc_ctx.plugin_stopped(view_id, plugin, code);
        self.doc_ctx.update_cmds(view_id, plugin, &Vec::new());
    }
}

fn n_spaces(n: usize) -> &'static str {
    let spaces = "                                ";
    assert!(n <= spaces.len());
    &spaces[..n]
}
