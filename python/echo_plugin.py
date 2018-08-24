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

from xi_plugin import start_plugin, Plugin


class Echo(Plugin):
    """Echoes back the active buffer at each update.

    This is a very simple sample plugin which demonstrates (and helps debug)
    the line cache.
    """

    def update(self, peer, rev, start, end,
               new_len, edit_type, author, text=None):
        header = "### BUFFER REV {} LEN {} ###".format(rev, self.lines.total_bytes)
        contents = "\n".join(self.lines)
        footer = '#' * len(header)
        self.print_err("\n{}\n{}\n{}".format(header, contents, footer))
        return 1


def main():
    start_plugin(Echo())

if __name__ == "__main__":
    main()
