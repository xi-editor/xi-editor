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
from collections import namedtuple


Selection = namedtuple('Selection', ['start', 'end', 'is_caret'])


class View(object):
    """Represents a view into a buffer."""
    def __init__(self, view_id, lines):
        self.view_id = view_id
        self.lines = lines

    @property
    def path(self):
        return self.lines.path

    @property
    def syntax(self):
        return self.lines.syntax

    def get_selections(self):
        selections = self.lines.peer.get_selections(self.view_id)
        selections = selections['selections']
        return [Selection(s, e, (s == e)) for (s, e) in selections]

    def update_spans(self, *args, **kwargs):
        self.lines.peer.update_spans(self.view_id, *args, **kwargs)

    def add_scopes(self, *args, **kwargs):
        self.lines.peer.add_scopes(self.view_id, *args, **kwargs)

    def edit(self, *args, **kwargs):
        self.lines.peer.edit(self.view_id, *args, **kwargs)
