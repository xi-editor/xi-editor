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

from __future__ import print_function
from __future__ import unicode_literals


class Edit(object):
    """A convenience class for describing an edit action."""
    def __init__(self, rev, insert_range, new_text, after_cursor=False):
        self.rev = rev
        self.start, self.end = insert_range
        self.text = new_text
        self.after_cursor = after_cursor

    def to_dict(self):
        return {
            "rev": self.rev,
            "start": self.start,
            "end": self.end,
            "after_cursor": self.after_cursor,
            "text": self.text
        }

    def __repr__(self):
        return repr(self.to_dict())

