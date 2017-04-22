#!/usr/bin/env python3

# Copyright 2017 Google Inc. All rights reserved.
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

import os
from xi_plugin import start_plugin, Plugin

try:
    import enchant
except ImportError:
    import sys
    print("spellcheck plugin requires pyenchant: https://github.com/rfk/pyenchant",
          file=sys.stderr, flush=True)
    sys.exit(1)


class Spellcheck(Plugin):
    """Basic spellcheck using pyenchant."""
    def __init__(self):
        super(Spellcheck, self).__init__()
        lang = os.environ.get("LC_CTYPE", "en_US.utf-8").split('.')[0]
        self.dictionary = enchant.Dict(lang)
        self.print_err("loaded dictionary for {}".format(lang))
        self.in_word = False

    def update(self, peer, author, rev, start, end,
               new_len, edit_type, text=None):
        if author == self.identifier:
            pass
        elif not self.in_word and text.isalpha():
            self.in_word = True
            # punctuation not exhaustive, this is a demo ;)
        elif self.in_word and (text.isspace() or text in ["!", ",", ".", ":", ";", "?"]):
            self.in_word = False
            line, col = self.lines.linecol_for_offset(end)
            prev_word = self.lines.previous_word(end)
            # TODO: libs should provide some "Text" object, which represents some string,
            # and provides convenience methods for getting relevant offsets, setting styles, etc
            if prev_word and not self.dictionary.check(prev_word):
                spans = [{'start': 0, 'end': len(prev_word), 'fg': 0xfff00000, 'font': 1}]
                peer.set_fg_spans(end-len(prev_word), len(prev_word), spans, rev)
        return 0


def main():
    start_plugin(Spellcheck())


if __name__ == "__main__":
    main()
