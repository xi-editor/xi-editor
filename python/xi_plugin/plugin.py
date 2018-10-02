# Copyright 2017 The xi-editor Authors.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http:#www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

import sys

from . import edit

PLUGIN_ACK_OK = 1


class Plugin(object):
    def __init__(self):
        self.identifier = type(self).__name__

    def print_err(self, err):
        print("PLUGIN.PY {}>>> {}".format(self.identifier, err), file=sys.stderr)
        sys.stderr.flush()

    def new_edit(self, rev, edit_range, new_text,
                 priority=edit.EDIT_PRIORITY_NORMAL, after_cursor=False):
        return edit.Edit(rev, edit_range, new_text,
                         self.identifier, priority, after_cursor)

    def initialize(self, view):
        self.print_err("initialize: {}".format(view.view_id))
        pass

    def did_save(self, view, old_path):
        self.print_err("did_save: {}".format(view.view_id))
        pass

    # TODO: some better internal representation of a delta
    # Maybe an 'Update' type or a 'Text' type or both
    def update(self, view, start, end, new_len, rev, edit_type, author, text):
        self.print_err("update: {}".format(view.view_id))
        return PLUGIN_ACK_OK

    def shutdown(self):
        self.print_err("shutdown")
        pass


class GlobalPlugin(Plugin):

    def initialize(self, views):
        view_ids = ', '.join(v.view_id for v in views)
        self.print_err("initialize global: {}".format(view_ids))

    def new_buffer(self, view):
        self.print_err("new_buffer: {}".format(view.view_id))

    def did_close(self, view_id):
        self.print_err("did_close: {}".format(view_id))
