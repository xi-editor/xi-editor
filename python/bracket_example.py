#!/usr/bin/env python3

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

from xi_plugin import start_plugin, Plugin, edit

MATCHES = {"{": "}", "[": "]", "(": ")"}


class BracketCloser(Plugin):
    """Naively closes opened brackets, parens, & braces."""

    def update(self, view, author, rev, start, end,
               new_len, edit_type, text=None):
        resp = 0
        close_char = MATCHES.get(text)
        if close_char:
            # compute a delta from params:
            new_cursor = end + new_len
            # we set 'after_cursor' because we want the edit to appear to the right
            # of the active cursor. we set priority=HIGH because we want this edit
            # applied after concurrent edits.
            resp = self.new_edit(rev, (new_cursor, new_cursor), close_char,
                                 after_cursor=True, priority=edit.EDIT_PRIORITY_HIGH)
        return resp


def main():
    start_plugin(BracketCloser())


if __name__ == "__main__":
    main()
