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

import time

from xi_plugin import start_plugin, Plugin


class Shouty(Plugin):
    """Replaces lowercase input with uppercase input."""

    def update(self, peer, author, rev, start, end,
               new_len, edit_type, text=None):
        resp = 0
        if not (author == self.identifier) and text and text.isalpha():
            text = text.upper()
            # compute a delta from params:
            resp = self.new_edit(rev, (start, end + len(text)) , text)
            # time.sleep(0.1)
        return resp


def main():
    start_plugin(Shouty())


if __name__ == "__main__":
    main()
