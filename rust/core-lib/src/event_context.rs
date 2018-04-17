// Copyright 2018 Google Inc. All rights reserved.
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

//! A container for the state relevant to a single event.

use std::cell::RefCell;
use std::iter;
use std::path::Path;

use serde_json::{self, Value};

use xi_rope::Rope;
use xi_rope::interval::Interval;
use xi_rope::rope::LinesMetric;
use xi_rpc::{RemoteError, Error as RpcError};
use xi_trace::trace_block;

use rpc::{EditNotification, EditRequest, LineRange};
use plugins::rpc::{PluginBufferInfo, PluginNotification, PluginRequest,
PluginUpdate, UpdateResponse};

use styles::ThemeStyleMap;
use config::{BufferConfig, ConfigManager};

use WeakXiCore;
use tabs::{ViewId, PluginId};
use editor::Editor;
use file::FileInfo;
use edit_types::{EventDomain, SpecialEvent};
use client::Client;
use plugins::Plugin;
use selection::SelRegion;
use view::View;

// Maximum returned result from plugin get_data RPC.
pub const MAX_SIZE_LIMIT: usize = 1024 * 1024;

/// A collection of all the state relevant for handling a particular event.
///
/// This is created dynamically for each event that arrives to the core,
/// such as a user-initiated edit or style updates from a plugin.
pub struct EventContext<'a> {
    pub(crate) editor: &'a RefCell<Editor>,
    pub(crate) info: Option<&'a FileInfo>,
    pub(crate) view: &'a RefCell<View>,
    pub(crate) siblings: Vec<&'a RefCell<View>>,
    pub(crate) plugins: Vec<&'a Plugin>,
    pub(crate) client: &'a Client,
    pub(crate) style_map: &'a RefCell<ThemeStyleMap>,
    pub(crate) weak_core: &'a WeakXiCore,
}

impl<'a> EventContext<'a> {
    /// Executes a closure with mutable references to the editor and the view,
    /// common in edit actions that modify the text.
    pub(crate) fn with_editor<R, F>(&mut self, f: F) -> R
        where F: FnOnce(&mut Editor, &mut View) -> R
    {
        let mut editor = self.editor.borrow_mut();
        let mut view = self.view.borrow_mut();
        f(&mut editor, &mut view)
    }

    /// Executes a closure with a mutable reference to the view and a reference
    /// to the current text. This is common to most edits that just modify
    /// selection or viewport state.
    fn with_view<R, F>(&mut self, f: F) -> R
        where F: FnOnce(&mut View, &Rope) -> R
    {
        let editor = self.editor.borrow();
        let mut view = self.view.borrow_mut();
        f(&mut view, editor.get_buffer())
    }

    pub(crate) fn do_edit(&mut self, cmd: EditNotification) {
        use self::EventDomain as E;
        let event: EventDomain = cmd.into();
        match event {
            E::View(cmd) => self.with_view(
                |view, text| view.do_edit(text, cmd)),
            E::Buffer(cmd) => self.with_editor(
                |ed, view| ed.do_edit(view, cmd)),
            E::Special(cmd) => self.do_special(cmd),
        }
        self.after_edit("core");
        self.render();
    }

    fn do_special(&mut self, cmd: SpecialEvent) {
        match cmd {
            SpecialEvent::DebugRewrap => (),
            SpecialEvent::DebugPrintSpans => self.with_editor(
                |ed, view| {
                    let sel = view.sel_regions().last().unwrap();
                    let iv = Interval::new_closed_open(sel.min(), sel.max());
                    ed.get_layers().debug_print_spans(iv);
                }),
            SpecialEvent::RequestLines(LineRange { first, last }) =>
                self.do_request_lines(first as usize, last as usize),
        }
    }

    pub(crate) fn do_edit_sync(&mut self, cmd: EditRequest
                               ) -> Result<Value, RemoteError> {
        use self::EditRequest::*;
        let result = match cmd {
            Cut => Ok(self.with_editor(|ed, view| ed.do_cut(view))),
            Copy => Ok(self.with_editor(|ed, view| ed.do_copy(view))),
            Find { chars, case_sensitive } => Ok(self.with_view(
                |view, text| view.do_find(text, chars, case_sensitive))),
        };
        self.after_edit("core");
        self.render();
        result
    }

    pub(crate) fn do_plugin_cmd(&mut self, plugin: PluginId,
                                 cmd: PluginNotification) {
        use self::PluginNotification::*;
        match cmd {
            AddScopes { scopes } => {
                let mut ed = self.editor.borrow_mut();
                let style_map = self.style_map.borrow();
                ed.get_layers_mut().add_scopes(plugin, scopes, &style_map);
            }
            UpdateSpans { start, len, spans, rev } => self.with_editor(
                |ed, view| ed.update_spans(view, plugin, start,
                                           len, spans, rev)),
            Edit { edit } => self.with_editor(
                |ed, _| ed.apply_plugin_edit(edit, None)),
            Alert { msg } => self.client.alert(&msg),
        };
        self.after_edit(&plugin.to_string());
        self.render();
    }

    pub(crate) fn do_plugin_cmd_sync(&mut self, _plugin: PluginId,
                                      cmd: PluginRequest) -> Value {
        use self::PluginRequest::*;
        match cmd {
            LineCount =>
                json!(self.editor.borrow().plugin_n_lines()),
            GetData { start, unit, max_size, rev } =>
                json!(self.editor.borrow()
                      .plugin_get_data(start, unit, max_size, rev)),
            GetSelections =>
                json!("not implemented"),
        }
    }

    /// Commits any changes to the buffer, updating views and plugins as needed.
    /// This only updates internal state; it does not update the client.
    fn after_edit(&mut self, author: &str) {
        let _t = trace_block("EventContext::after_edit", &["core"]);
        let mut ed = self.editor.borrow_mut();
        let (delta, last_text, keep_sels) = match ed.commit_delta() {
            Some(edit_info) => edit_info,
            None => return,
        };
        let iter_views = iter::once(&self.view).chain(self.siblings.iter());
        iter_views.for_each(|view| view.borrow_mut()
                            .after_edit(ed.get_buffer(), &last_text,
                                        &delta, ed.is_pristine(),
                                        keep_sels));

        let new_len = delta.new_document_len();
        let nb_lines = ed.get_buffer().measure::<LinesMetric>() + 1;
        // don't send the actual delta if it is too large, by some heuristic
        let approx_size = delta.inserts_len() + (delta.els.len() * 10);
        let delta = if approx_size > MAX_SIZE_LIMIT { None } else { Some(delta) };

        let update = PluginUpdate::new(
                self.view.borrow().view_id,
                ed.get_head_rev_token(),
                delta,
                new_len,
                nb_lines,
                ed.get_edit_type().to_owned(),
                author.into());
        let undo_group = ed.get_active_undo_group();

        // we always increment and decrement regardless of whether we're
        // sending plugins, to ensure that GC runs.
        ed.increment_revs_in_flight();

        self.plugins.iter().for_each(|plugin| {
            ed.increment_revs_in_flight();
            let weak_core = self.weak_core.clone();
            let id = plugin.id;
            let view_id = self.view.borrow().view_id;
            plugin.update(&update, move |resp| {
                weak_core.handle_plugin_update(id, view_id, undo_group, resp);
            });
        });
        ed.dec_revs_in_flight();
        ed.update_edit_type();
    }

    /// Flushes any changes in the views out to the frontend.
    pub(crate) fn render(&mut self) {
        let _t = trace_block("EventContext::render", &["core"]);
        let ed = self.editor.borrow();
        //TODO: render other views
        self.view.borrow_mut()
            .render_if_dirty(ed.get_buffer(), self.client, self.style_map,
                             ed.get_layers().get_merged())
    }
}

/// Helpers related to specific commands.
///
/// Certain events and actions don't generalize well; handling these
/// requires access to particular combinations of state. We isolate such
/// special cases here.
impl<'a> EventContext<'a> {
    pub(crate) fn finish_init(&mut self) {
        let ed = self.editor.borrow();
        let config = ed.get_config().to_table();
        self.client.config_changed(self.view.borrow().view_id, &config);
        self.render();
        // notify plugins
        if !self.plugins.is_empty() {
            let info = self.plugin_info();
            self.plugins.iter().for_each(|plugin| plugin.new_buffer(&info));
        }
    }

    pub(crate) fn after_save(&mut self, path: &Path, new_config: BufferConfig) {
        // notify plugins
        let view_id = self.view.borrow().view_id;
        self.plugins.iter().for_each(
            |plugin| plugin.did_save(view_id, path)
            );
        if let Some(changes) = self.editor.borrow_mut().set_config(new_config) {
            self.client.config_changed(view_id, &changes);
        }
        self.editor.borrow_mut().set_pristine();
        self.with_view(|view, text| view.set_dirty(text));
        self.render()
    }

    /// Returns `true` if this was the last view
    pub(crate) fn close_view(&self) -> bool {
        // we probably want to notify plugins _before_ we close the view
        // TODO: determine what plugins we're stopping
        let view_id = self.view.borrow().view_id;
        self.plugins.iter().for_each(|plug| plug.close_view(view_id));
        self.siblings.is_empty()
    }

    pub(crate) fn config_changed(&mut self, config_manager: &ConfigManager) {
        {
            let mut ed = self.editor.borrow_mut();
            let mut view = self.view.borrow_mut();
            let syntax = ed.get_syntax().to_owned();
            let new_config = config_manager.get_buffer_config(syntax,
                                                              view.buffer_id);
            if let Some(changes) = ed.set_config(new_config) {
                if changes.contains_key("wrap_width") {
                    let wrap_width = ed.get_config().items.wrap_width;
                    view.rewrap(&ed.get_buffer(), wrap_width);
                    view.set_dirty(&ed.get_buffer());
                }
                self.client.config_changed(view.view_id, &changes);
            }
        }
        self.render()
    }

    pub(crate) fn reload(&mut self, text: Rope) {
        self.with_editor(|ed, view| {
            let new_len = text.len();
            view.collapse_selections(ed.get_buffer());
            view.unset_find(ed.get_buffer());
            let prev_sel = view.sel_regions().first().map(|s| s.clone());
            ed.reload(text);
            if let Some(prev_sel) = prev_sel {
                let offset = prev_sel.start.min(new_len);
                let sel = SelRegion::caret(offset);
                view.set_selection(ed.get_buffer(), sel);
            }
        });

        self.after_edit("core");
        self.render();
    }

    pub(crate) fn plugin_info(&mut self) -> PluginBufferInfo {
        let ed = self.editor.borrow();
        let nb_lines = ed.get_buffer().measure::<LinesMetric>() + 1;
        let views: Vec<ViewId> = iter::once(&self.view)
            .chain(self.siblings.iter())
            .map(|v| v.borrow().view_id)
            .collect();
        let buffer_id = self.view.borrow().buffer_id;

        let config = ed.get_config().to_table();
        let path = self.info.map(|info| info.path.to_owned());
        PluginBufferInfo::new(buffer_id, &views,
                              ed.get_head_rev_token(),
                              ed.get_buffer().len(), nb_lines,
                              path,
                              ed.get_syntax().clone(),
                              config)

    }

    // TODO: remove support for sync updates
    pub(crate) fn do_plugin_update(&mut self, update: Result<Value, RpcError>,
                                    undo_group: usize) {

        match update.map(serde_json::from_value::<UpdateResponse>) {
            Ok(Ok(UpdateResponse::Edit(edit))) => self.editor.borrow_mut()
                .apply_plugin_edit(edit, Some(undo_group)),
            Ok(Ok(UpdateResponse::Ack(_))) => (),
            Ok(Err(err)) => eprintln!("plugin response json err: {:?}", err),
            Err(err) => eprintln!("plugin shutdown, do something {:?}", err),
        }
        self.editor.borrow_mut().dec_revs_in_flight();
    }

    fn do_request_lines(&mut self, first: usize, last: usize) {
        let mut view = self.view.borrow_mut();
        let ed = self.editor.borrow();
        view.request_lines(ed.get_buffer(), self.client, self.style_map,
                           ed.get_layers().get_merged(), first, last)

    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use core::dummy_weak_core;
    use tabs::BufferId;
    use xi_rpc::test_utils::DummyPeer;

    struct ContextHarness {
        view: RefCell<View>,
        editor: RefCell<Editor>,
        client: Client,
        core_ref: WeakXiCore,
        style_map: RefCell<ThemeStyleMap>,
    }

    impl ContextHarness {
        fn new<S: AsRef<str>>(s: S) -> Self {
            let view_id = ViewId(1);
            let buffer_id = BufferId(2);
            let config_manager = ConfigManager::default();
            let view = RefCell::new(View::new(view_id, buffer_id));
            let editor = RefCell::new(
                Editor::with_text(s, config_manager.default_buffer_config()));
            let client = Client::new(Box::new(DummyPeer));
            let core_ref = dummy_weak_core();
            let style_map = RefCell::new(ThemeStyleMap::new());
            ContextHarness { view, editor, client, core_ref, style_map }
        }

        /// Renders the text and selections. cursors are represented with
        /// the pipe '|', and non-caret regions are represented by \[braces\].
        fn debug_render(&self) -> String {
            let b = self.editor.borrow();
            let mut text: String = b.get_buffer().into();
            let v = self.view.borrow();
            for sel in v.sel_regions().iter().rev() {
                if sel.end == sel.start {
                    text.insert(sel.end, '|');
                } else if sel.end > sel.start {
                    text.insert_str(sel.end, "|]");
                    text.insert(sel.start, '[');
                } else {
                    text.insert(sel.start, ']');
                    text.insert_str(sel.end, "[|");
                }
            }
            text
        }

        fn make_context<'a>(&'a self) -> EventContext<'a> {
            EventContext {
                view: &self.view,
                editor: &self.editor,
                info: None,
                siblings: Vec::new(),
                plugins: Vec::new(),
                client: &self.client,
                style_map: &self.style_map,
                weak_core: &self.core_ref,
            }
        }
    }

    #[test]
    fn smoke_test() {
        let harness = ContextHarness::new("");
        let mut ctx = harness.make_context();
        ctx.do_edit(EditNotification::Insert { chars: "hello".into() });
        ctx.do_edit(EditNotification::Insert { chars: " ".into() });
        ctx.do_edit(EditNotification::Insert { chars: "world".into() });
        ctx.do_edit(EditNotification::Insert { chars: "!".into() });
        assert_eq!(harness.debug_render(),"hello world!|");
        ctx.do_edit(EditNotification::MoveWordLeft);
        ctx.do_edit(EditNotification::InsertNewline);
        assert_eq!(harness.debug_render(),"hello \n|world!");
        ctx.do_edit(EditNotification::MoveWordRightAndModifySelection);
        assert_eq!(harness.debug_render(), "hello \n[world|]!");
        ctx.do_edit(EditNotification::Insert { chars: "friends".into() });
        assert_eq!(harness.debug_render(), "hello \nfriends|!");
    }

     #[test]
    fn simple_indent_outdent_test() {
        let harness = ContextHarness::new("");
        let mut ctx = harness.make_context();
        // Single indent and outdent test
        ctx.do_edit(EditNotification::Insert { chars: "hello".into() });
        ctx.do_edit(EditNotification::Indent);
        assert_eq!(harness.debug_render(),"    hello|");
        ctx.do_edit(EditNotification::Outdent);
        assert_eq!(harness.debug_render(),"hello|");
        // Non-selection one line indent and outdent test
        ctx.do_edit(EditNotification::Indent);
        ctx.do_edit(EditNotification::InsertNewline);
        ctx.do_edit(EditNotification::Insert { chars: "world".into() });
        assert_eq!(harness.debug_render(),"    hello\nworld|");
        ctx.do_edit(EditNotification::MoveWordLeft);
        ctx.do_edit(EditNotification::MoveToBeginningOfDocumentAndModifySelection);
        ctx.do_edit(EditNotification::Indent);
        assert_eq!(harness.debug_render(),"    [|    hello\n]world");
        ctx.do_edit(EditNotification::Outdent);
        assert_eq!(harness.debug_render(),"[|    hello\n]world");
        
    }
}
