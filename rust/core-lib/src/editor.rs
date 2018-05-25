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
use std::cmp::min;
use std::collections::BTreeSet;

use serde_json::Value;

use xi_rope::rope::{Rope, RopeInfo, LinesMetric};
use xi_rope::interval::Interval;
use xi_rope::delta::{self, Delta, Transformer};
use xi_rope::engine::{Engine, RevId, RevToken};
use xi_rope::spans::SpansBuilder;
use xi_trace::trace_block;

use config::{BufferConfig, Table};
use event_context::MAX_SIZE_LIMIT;
use edit_types::BufferEvent;
use layers::Layers;
use movement::{Movement, region_movement};
use plugins::PluginId;
use plugins::rpc::{PluginEdit, ScopeSpan, TextUnit, GetDataResponse};
use selection::{Selection, SelRegion};
use styles::ThemeStyleMap;
use syntax::LanguageId;
use view::View;

#[cfg(not(feature = "ledger"))]
pub struct SyncStore;
#[cfg(feature = "ledger")]
use fuchsia::sync::SyncStore;

// TODO This could go much higher without issue but while developing it is
// better to keep it low to expose bugs in the GC during casual testing.
const MAX_UNDOS: usize = 20;

enum IndentDirection {
    In,
    Out
}

pub struct Editor {
    /// The contents of the buffer.
    text: Rope,
    /// The CRDT engine, which tracks edit history and manages concurrent edits.
    engine: Engine,

    /// The most recent revision.
    last_rev_id: RevId,
    /// The revision of the last save.
    pristine_rev_id: RevId,
    undo_group_id: usize,
    /// Undo groups that may still be toggled
    live_undos: Vec<usize>,
    /// The index of the current undo; subsequent undos are currently 'undone'
    /// (but may be redone)
    cur_undo: usize,
    /// undo groups that are undone
    undos: BTreeSet<usize>,
    /// undo groups that are no longer live and should be gc'ed
    gc_undos: BTreeSet<usize>,

    this_edit_type: EditType,
    last_edit_type: EditType,

    revs_in_flight: usize,

    /// Used only on Fuchsia for syncing
    #[allow(dead_code)]
    sync_store: Option<SyncStore>,
    #[allow(dead_code)]
    last_synced_rev: RevId,

    syntax: LanguageId,
    layers: Layers,
    config: BufferConfig,
}

impl Editor {
    /// Creates a new `Editor` with a new empty buffer.
    pub fn new(config: BufferConfig) -> Editor {
        Self::with_text("", config)
    }

    /// Creates a new `Editor`, loading text into a new buffer.
    pub fn with_text<T>(text: T, config: BufferConfig) -> Editor
        where T: Into<Rope>,
    {

        let engine = Engine::new(text.into());
        let buffer = engine.get_head().clone();
        let last_rev_id = engine.get_head_rev_id();

        Editor {
            text: buffer,
            syntax: LanguageId::default(),
            engine,
            last_rev_id,
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
            layers: Layers::default(),
            config,
            revs_in_flight: 0,
            sync_store: None,
            last_synced_rev: last_rev_id,
        }
    }

    pub(crate) fn get_buffer(&self) -> &Rope {
        &self.text
    }

    pub(crate) fn get_layers(&self) -> &Layers {
        &self.layers
    }

    pub(crate) fn get_layers_mut(&mut self) -> &mut Layers {
        &mut self.layers
    }

    pub(crate) fn get_head_rev_token(&self) -> u64 {
        self.engine.get_head_rev_id().token()
    }

    pub(crate) fn get_edit_type(&self) -> &str {
        self.this_edit_type.json_string()
    }

    pub(crate) fn get_active_undo_group(&self) -> usize {
        *self.live_undos.last().unwrap_or(&0)
    }

    pub(crate) fn update_edit_type(&mut self) {
        self.last_edit_type = self.this_edit_type;
        self.this_edit_type = EditType::Other
    }

    pub(crate) fn set_pristine(&mut self) {
        self.pristine_rev_id = self.engine.get_head_rev_id();
    }

    pub(crate) fn is_pristine(&self) -> bool {
        self.engine.is_equivalent_revision(self.pristine_rev_id,
                                           self.engine.get_head_rev_id())
    }

    /// Sets this Editor's contents to `text`, preserving undo state and cursor
    /// position when possible.
    pub fn reload(&mut self, text: Rope) {
        let mut builder = delta::Builder::new(self.text.len());
        let all_iv = Interval::new_closed_open(0, self.text.len());
        builder.replace(all_iv, text);
        self.add_delta(builder.build());
        self.set_pristine();
    }

    /// Sets the config for this buffer. If the new config differs
    /// from the existing config, returns the modified items.
    pub fn set_config(&mut self, conf: BufferConfig) -> Option<Table> {
        if let Some(changes) = conf.changes_from(Some(&self.config)) {
            self.config = conf;
            Some(changes)
        } else {
            None
        }
    }

    pub fn get_config(&self) -> &BufferConfig {
        &self.config
    }

    /// Returns this `Editor`'s active `LanguageId`.
    pub fn get_syntax(&self) -> &LanguageId {
        &self.syntax
    }

    //TODO: temporary, this should be tracked in config manager
    pub(crate) fn set_syntax(&mut self, language: LanguageId) {
        self.syntax = language;
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

    fn insert<T>(&mut self, view: &View, text: T)
        where T: Into<Rope>
    {
        let rope = text.into();
        let mut builder = delta::Builder::new(self.text.len());
        for region in view.sel_regions() {
            let iv = Interval::new_closed_open(region.min(), region.max());
            builder.replace(iv, rope.clone());
        }
        self.add_delta(builder.build());
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

        if self.this_edit_type == self.last_edit_type
            && self.this_edit_type != EditType::Other
            && self.this_edit_type != EditType::Transpose
            && !self.live_undos.is_empty()
        {
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

    /// generates a delta from a plugin's response and applies it to the buffer.
    pub fn apply_plugin_edit(&mut self, edit: PluginEdit) {
        let _t = trace_block("Editor::apply_plugin_edit", &["core"]);
        let undo_group = edit.undo_group;
        if let Some(undo_group) = undo_group {
            // non-async edits modify their associated revision
            //TODO: get priority working, so that plugin edits don't
            // necessarily move cursor
            self.engine.edit_rev(edit.priority as usize, undo_group,
                                 edit.rev, edit.delta);
            self.text = self.engine.get_head().clone();
        }
        else {
            self.add_delta(edit.delta);
        }
    }

    /// Commits the current delta. If the buffer has changed, returns
    /// a 3-tuple containing the delta representing the changes, the previous
    /// buffer, and a bool indicating whether selections should be preserved.
    pub(crate) fn commit_delta(&mut self)
        -> Option<(Delta<RopeInfo>, Rope, bool)> {
        let _t = trace_block("Editor::commit_delta", &["core"]);

        if self.engine.get_head_rev_id() == self.last_rev_id {
            return None;
        }

        let last_token = self.last_rev_id.token();
        let delta = self.engine.delta_rev_head(last_token);
        // TODO (performance): it's probably quicker to stash last_text
        // rather than resynthesize it.
        let last_text = self.engine.get_rev(last_token)
            .expect("last_rev not found");

        let keep_selections = self.this_edit_type == EditType::Transpose;
        self.layers.update_all(&delta);

        self.last_rev_id = self.engine.get_head_rev_id();
        self.sync_state_changed();
        Some((delta, last_text, keep_selections))
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

    pub fn merge_new_state(&mut self, new_engine: Engine) {
        self.engine.merge(&new_engine);
        self.text = self.engine.get_head().clone();
        // TODO: better undo semantics. This only implements separate undo
        // histories for low concurrency.
        self.undo_group_id = self.engine.max_undo_group_id() + 1;
        self.last_synced_rev = self.engine.get_head_rev_id();
        self.commit_delta();
        //self.render();
        //FIXME: render after fuchsia sync
    }

    /// See `Engine::set_session_id`. Only useful for Fuchsia sync.
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

    fn delete_backward(&mut self, view: &View) {
        // TODO: this function is workable but probably overall code complexity
        // could be improved by implementing a "backspace" movement instead.
        let mut builder = delta::Builder::new(self.text.len());
        for region in view.sel_regions() {
            let start = if !region.is_caret() {
                region.min()
            } else {
                // backspace deletes max(1, tab_size) contiguous spaces
                let (_, c) = view.offset_to_line_col(&self.text,
                                                     region.start);
                let use_spaces = self.config.items.translate_tabs_to_spaces;
                let use_tab_stops = self.config.items.use_tab_stops;
                let tab_size = self.config.items.tab_size;
                let tab_off = c & tab_size;
                let tab_size = if tab_off == 0 { tab_size } else { tab_off };
                let preceded_by_spaces = region.start > 0 &&
                    (region.start.saturating_sub(tab_size)..region.start)
                    .all(|i| self.text.byte_at(i) == b' ');
                if preceded_by_spaces && use_spaces && use_tab_stops {
                    region.start - tab_size
                } else {
                    self.text.prev_grapheme_offset(region.end)
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

    /// Common logic for a number of delete methods. For each region in the
    /// selection, if the selection is a caret, delete the region between
    /// the caret and the movement applied to the caret, otherwise delete
    /// the region.
    ///
    /// If `save` is set, save the deleted text into the kill ring.
    fn delete_by_movement(&mut self, view: &View, movement: Movement,
                          save: bool, kill_ring: &mut Rope) {
        // We compute deletions as a selection because the merge logic
        // is convenient. Another possibility would be to make the delta
        // builder able to handle overlapping deletions (with union semantics).
        let mut deletions = Selection::new();
        for &r in view.sel_regions() {
            if r.is_caret() {
                let new_region = region_movement(movement, r, view,
                                                 &self.text, true);
                deletions.add_region(new_region);
            } else {
                deletions.add_region(r);
            }
        }
        if save {
            let saved = self.extract_sel_regions(&deletions)
                .unwrap_or_default();
            *kill_ring = saved.into();
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

    /// Extracts non-caret selection regions into a string,
    /// joining multiple regions with newlines.
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

    fn insert_newline(&mut self, view: &View) {
        self.this_edit_type = EditType::InsertChars;
        let text = self.config.items.line_ending.clone();
        self.insert(view, &text);
    }

    fn insert_tab(&mut self, view: &View) {
        let mut builder = delta::Builder::new(self.text.len());
        let const_tab_text = self.get_tab_text(self.config.items.tab_size);

        for region in view.sel_regions() {
            let line_range = view.get_line_range(&self.text, region);

            if line_range.len() > 1 {
                for line in line_range {
                    let offset = view.line_col_to_offset(&self.text, line, 0);
                    let iv = Interval::new_closed_open(offset, offset);
                    builder.replace(iv, Rope::from(const_tab_text));
                }
            } else {
                let (_, col) = view.offset_to_line_col(&self.text, region.start);
                let mut tab_size = self.config.items.tab_size;
                tab_size = tab_size - (col % tab_size);
                let tab_text = self.get_tab_text(tab_size);

                let iv = Interval::new_closed_open(region.min(), region.max());
                builder.replace(iv, Rope::from(tab_text));
            }
        }
        self.this_edit_type = EditType::InsertChars;
        self.add_delta(builder.build());

        // What follows is old indent code, retained because it will be useful for
        // indent action (Sublime no longer does indent on non-caret selections).
        /*
            let (first_line, _) = view.offset_to_line_col(&self.text, view.sel_min());
            let (last_line, last_col) =
                view.offset_to_line_col(&self.text, view.sel_max());
            let last_line = if last_col == 0 && last_line > first_line {
                last_line
            } else {
                last_line + 1
            };
            for line in first_line..last_line {
                let offset = view.line_col_to_offset(&self.text, line, 0);
                let iv = Interval::new_closed_open(offset, offset);
                self.add_simple_edit(iv, Rope::from(n_spaces(TAB_SIZE)));
            }
        */
    }

    /// Indents or outdents lines based on selection and user's tab settings.
    /// Uses a BTreeSet to holds the collection of lines to modify.
    /// Preserves cursor position and current selection as much as possible.
    /// Tries to have behavior consistent with other editors like Atom,
    /// Sublime and VSCode, with non-caret selections not being modified.
    fn modify_indent(&mut self, view: &View, direction: IndentDirection) {
        let mut lines = BTreeSet::new();
        let tab_text = self.get_tab_text(self.config.items.tab_size);
        for region in view.sel_regions() {
            let line_range = view.get_line_range(&self.text, region);
            for line in line_range {
                lines.insert(line);
            }
        }
        match direction {
            IndentDirection::In =>  self.indent(view, lines, tab_text),
            IndentDirection::Out => self.outdent(view, lines, tab_text)
         };

    }

    fn indent(&mut self, view: &View, lines: BTreeSet<usize>, tab_text: &str) {
        let mut builder = delta::Builder::new(self.text.len());
        for line in lines {
            let offset = view.line_col_to_offset(&self.text, line, 0);
            let interval = Interval::new_closed_open(offset, offset);
            builder.replace(interval, Rope::from(tab_text));

        }
        self.this_edit_type = EditType::InsertChars;
        self.add_delta(builder.build());
    }

    fn outdent(&mut self, view: &View, lines: BTreeSet<usize>, tab_text: &str) {
        let mut builder = delta::Builder::new(self.text.len());
        for line in lines {
            let offset = view.line_col_to_offset(&self.text, line, 0);
            let tab_offset = view.line_col_to_offset(&self.text, line,
                                                     tab_text.len());
            let interval = Interval::new_closed_open(offset, tab_offset);
            let leading_slice = self.text.slice_to_string(interval.start(),
                                                          interval.end());
            if leading_slice == tab_text {
                builder.delete(interval);
            } else if let Some(first_char_col) = leading_slice.find(|c: char| !c.is_whitespace()) {
                let first_char_offset = view.line_col_to_offset(&self.text, line, first_char_col);
                let interval = Interval::new_closed_open(offset, first_char_offset);
                builder.delete(interval);
            }
        }
        self.this_edit_type = EditType::Delete;
        self.add_delta(builder.build());
    }

    fn get_tab_text(&self, tab_size: usize) -> &'static str {
        let tab_text = if self.config.items.translate_tabs_to_spaces {
            n_spaces(tab_size)
        } else { "\t" };

        tab_text
    }

    // TODO: insert from keyboard or input method shouldn't break undo group,
    // but paste should.
    fn do_insert(&mut self, view: &View, chars: &str) {
        self.this_edit_type = EditType::InsertChars;
        self.insert(view, chars);
    }

    pub(crate) fn do_cut(&mut self, view: &mut View) -> Value {
        let result = self.do_copy(view);
        // This copy is just to make the borrow checker happy, could be optimized.
        let deletions = view.sel_regions().to_vec();
        self.delete_sel_regions(&deletions);
        result
    }

    pub(crate) fn do_copy(&self, view: &View) -> Value {
        if let Some(val) = self.extract_sel_regions(view.sel_regions()) {
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

    fn update_undos(&mut self) {
        self.engine.undo(self.undos.clone());
        self.text = self.engine.get_head().clone();
    }

    fn sel_region_to_interval_and_rope(&self, region: SelRegion) -> (Interval, Rope) {
        let as_interval = Interval::new_closed_open(region.min(), region.max());
        let interval_rope = Rope::from(self.text.slice_to_string(
            as_interval.start(), as_interval.end()));
        (as_interval, interval_rope)
    }

    fn do_transpose(&mut self, view: &View) {
        let mut builder = delta::Builder::new(self.text.len());
        let mut last = 0;
        let mut optional_previous_selection : Option<(Interval, Rope)> =
            last_selection_region(view.sel_regions()).map(
                |&region| self.sel_region_to_interval_and_rope(region));

        for &region in view.sel_regions() {
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

    fn yank(&mut self, view: &View, kill_ring: &mut Rope) {
        // TODO: if there are multiple cursors and the number of newlines
        // is one less than the number of cursors, split and distribute one
        // line per cursor.
        self.insert(view, kill_ring.clone());
    }

    fn transform_text<F: Fn(&str) -> String>(&mut self, view: &View,
                                             transform_function: F) {
        let mut builder = delta::Builder::new(self.text.len());

        for region in view.sel_regions() {
            let selected_text = self.text.slice_to_string(region.min(),
                                                          region.max());
            let interval = Interval::new_closed_open(region.min(), region.max());
            builder.replace(interval, Rope::from(transform_function(&selected_text)));
        }
        if !builder.is_empty() {
            self.this_edit_type = EditType::Other;
            self.add_delta(builder.build());
        }
    }

    pub(crate) fn do_edit(&mut self, view: &mut View, kill_ring: &mut Rope,
                          cmd: BufferEvent) {
        use self::BufferEvent::*;
        match cmd {
            Delete { movement, kill } =>
                self.delete_by_movement(view, movement, kill, kill_ring),
            Backspace => self.delete_backward(view),
            Transpose => self.do_transpose(view),
            Undo => self.do_undo(),
            Redo => self.do_redo(),
            Uppercase => self.transform_text(view, |s| s.to_uppercase()),
            Lowercase => self.transform_text(view, |s| s.to_lowercase()),
            Indent => self.modify_indent(view, IndentDirection::In),
            Outdent => self.modify_indent(view, IndentDirection::Out),
            InsertNewline => self.insert_newline(view),
            InsertTab => self.insert_tab(view),
            Insert(chars) => self.do_insert(view, &chars),
            Yank => self.yank(view, kill_ring),
        }
    }

    pub fn theme_changed(&mut self, style_map: &ThemeStyleMap) {
        self.layers.theme_changed(style_map);
    }

    pub fn plugin_n_lines(&self) -> usize {
        self.text.measure::<LinesMetric>() + 1
    }

    pub fn update_spans(&mut self, view: &mut View, plugin: PluginId,
                        start: usize, len: usize, spans: Vec<ScopeSpan>,
                        rev: RevToken) {
        let _t = trace_block("Editor::update_spans", &["core"]);
        // TODO: more protection against invalid input
        let mut start = start;
        let mut end_offset = start + len;
        let mut sb = SpansBuilder::new(len);
        for span in spans {
            sb.add_span(Interval::new_open_open(span.start, span.end),
                        span.scope_id);
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
        self.layers.update_layer(plugin, iv, spans);
        view.invalidate_styles(&self.text, start, end_offset);
    }

    pub fn plugin_get_data(&self, start: usize,
                           unit: TextUnit,
                           max_size: usize,
                           rev: RevToken) -> Option<GetDataResponse> {
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

fn last_selection_region(regions: &[SelRegion]) -> Option<&SelRegion> {
    for region in regions.iter().rev() {
        if !region.is_caret() {
            return Some(region);
        }
    }
    None
}

fn n_spaces(n: usize) -> &'static str {
    let spaces = "                                ";
    assert!(n <= spaces.len());
    &spaces[..n]
}
