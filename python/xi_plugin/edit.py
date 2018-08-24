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

import random

# thinking about ways to make things simpler for plugin authors
EDIT_PRIORITY_LOW =    0x100000
EDIT_PRIORITY_NORMAL = 0x1000000
EDIT_PRIORITY_HIGH =   0x10000000


class Edit(object):
    """A convenience class for describing an edit action."""
    def __init__(self, rev, insert_range, new_text, author,
                 priority=EDIT_PRIORITY_NORMAL, after_cursor=False):
        self.rev = rev
        self.start, self.end = insert_range
        self.text = new_text
        self.after_cursor = after_cursor
        self.priority = priority
        self.author = author

    # TODO: does this make sense
    # TODO: Also make sure this is deterministic, a given priority should have a constant relationship with core's default priority.
    # TODO: each plugin could be assigned a priority_factor on init which they apply to their edits instead of random bias
    def _smudge_priority(self, priority):
        """If priority is a default value, apply some random bias.

        Does not modify custom priorities.
        """
        if priority in [EDIT_PRIORITY_LOW, EDIT_PRIORITY_HIGH, EDIT_PRIORITY_NORMAL]:
            priority += random.randrange(0x10000)
        return priority

    def to_dict(self):
        return {
            "rev": self.rev,
            "start": self.start,
            "end": self.end,
            "after_cursor": self.after_cursor,
            "text": self.text,
            # TODO: names should be assigned by core.
            "author":  self.author,
            "priority": self._smudge_priority(self.priority),
        }

    def __repr__(self):
        return repr(self.to_dict())
