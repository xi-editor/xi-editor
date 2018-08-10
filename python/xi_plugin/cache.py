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

import bisect


class LineCache(object):
    """Basic access to the core's buffer.

    LineCache behaves like a list of lines. Individual lines can be accessed
    with the lines[idx] syntax. If a line is not present in the cache,
    it will be fetched, blocking the caller until it arrives.
    """
    def __init__(self, peer, buffer_id, views, buf_size, nb_lines, rev, syntax,
                 *, test_data=None, path=None):
        self.total_bytes = buf_size
        self.nb_lines = nb_lines
        # keep ends to make calculating offsets simple
        self.revision = rev
        self.path = path
        self.syntax = syntax
        self.view_id = views[0]
        self.peer = peer
        raw_data = test_data or peer.get_data(self.view_id, 0, self.revision)
        self.raw_lines = raw_data.splitlines(True) or ['']  # handle empty buffer
        self._recalculate_offsets()

    def _recalculate_offsets(self):
        self.offsets = [0] + [len(l) for l in self.raw_lines]
        tally = 0
        for idx in range(len(self.offsets)):
            new_tally = self.offsets[idx] + tally
            self.offsets[idx] += tally
            tally = new_tally

    def __len__(self):
        # TODO: to report our length, in lines, we have to go get all lines, which is bad
        while self.has_missing():
            self.get_data(self.peer, to_offset=self.total_bytes)
        return len(self.raw_lines)

    def __getitem__(self, idx):
        while idx >= len(self.raw_lines) and self.has_missing():
            self.get_data(self.peer, to_offset=self.total_bytes)
        # trim trailing newline
        return self.raw_lines[idx].rstrip('\n')

    def has_missing(self):
        """Returns true if the cache does not have a copy of the full buffer."""
        return self.offsets[-1] != self.total_bytes

    def linecol_for_offset(self, offset):
        """Given a byte offset, returns the corresponding line and column.

        Raises IndexError if the offset is out of bounds on the buffer.
        """
        if offset < 0 or offset > self.total_bytes:
            raise IndexError("offset {} invalid for buffer length {}".format(
                offset, self.total_bytes))
        if offset == 0:
            return (0, 0)
        if offset > self.offsets[-1]:
            self.get_data(offset, self.peer)

        line_nb = bisect.bisect_left(self.offsets, offset) - 1
        col = offset - self.offsets[line_nb]
        return (line_nb, col)

    def previous_word(self, offset):
        """Returns the word immediately preceding `offset`, or None.

        Word is defined as a "sequence of non-whitespace characters".
        If the offset is inside a word, the left half of the word will be returned.

        Returns None if no words precede `offset`.

        Raises `IndexError` if `offset` is out of bounds of the buffer.
        """

        line_nb, col = self.linecol_for_offset(offset)
        line = self[line_nb]
        word = line[:col].split()[-1]
        return word or None

    def apply_update(self, peer, author, rev, start, end, new_len, edit_type, text=None):
        # an update is bytes; can span multiple lines.
        text = text or ""
        # FIXME: this can fail on very large updates. In that case we should
        # clear raw_lines, update total_bytes, and maybe do a fetch?
        assert len(text) == new_len
        if end > self.total_bytes:
            self.get_data(peer, end)
        self.revision = rev
        self.total_bytes += (start-end) + new_len

        # cannot index past last offset (which is really the bounds of the buffer)
        first_changed_idx = bisect.bisect(self.offsets[:-1], start) - 1
        last_changed_idx = bisect.bisect(self.offsets[:-1], end) - 1

        orig_first_line = self.raw_lines[first_changed_idx]
        first_line_start = start - self.offsets[first_changed_idx]
        f_end = min(end - self.offsets[first_changed_idx], len(orig_first_line))
        first_line = ''.join((orig_first_line[:first_line_start], text, orig_first_line[f_end:]))

        # append remainder of last affected line, then recalculate lines in that interval
        if first_changed_idx != last_changed_idx:
            l_end = end - self.offsets[last_changed_idx]
            first_line += self.raw_lines[last_changed_idx][l_end:]
        new_lines = first_line.splitlines(True) or ['']

        # list[5:5] = [] is an insert without replacement
        self.offsets[first_changed_idx + 1:last_changed_idx + 2] = [0 for _ in range(len(new_lines))]
        # now update offsets:
        for idx in range(len(new_lines)):
            offset_idx = first_changed_idx + idx
            self.offsets[offset_idx + 1] = self.offsets[offset_idx] + len(new_lines[idx])

        # update all offsets after the edit
        offset_to_update = first_changed_idx + len(new_lines) + 1
        for idx in range(offset_to_update, len(self.offsets)):
            self.offsets[idx] += ((start-end) + new_len)

        self.raw_lines[first_changed_idx:last_changed_idx+1] = new_lines

    def get_data(self, peer, to_offset):
        while to_offset > self.offsets[-1]:
            raw_data = peer.get_data(self.view_id, self.offsets[-1], self.revision)
            raw_lines = raw_data.splitlines(True)

            # if current last line does not contain newline, append first new line directly
            if self.raw_lines[-1][-1] != '\n':
                self.raw_lines[-1] += raw_lines[0]
                self.offsets[-1] += len(raw_lines[0])
                raw_lines = raw_lines[1:]

            for idx in range(len(raw_lines)):
                prev_idx = len(self.offsets) - 1
                self.offsets.append(len(raw_lines[idx]) + self.offsets[prev_idx])
                self.raw_lines.extend(raw_lines)


# used to test data fetching
class MockPeer(object):

    def __init__(self, raw_data, max_size=32):
        self.raw_data = raw_data
        self.max_size = max_size

    def get_data(self, from_offset, rev, max_size=None):
        max_size = max_size or self.max_size

        if from_offset >= len(self.raw_data):
            return None
        end_offset = min(len(self.raw_data), from_offset + max_size)
        return self.raw_data[from_offset:end_offset]


def test_linebuffer_init():
    testdata = "this\nhas\nsome\nlines\nin it"
    cache = LineCache(len(testdata), None, 0, testdata)
    assert cache[0] == "this"
    assert cache[4] == "in it", "%s" % cache.raw_lines
    assert len(cache) == 5
    try:
        cache[5]
        assert False
    except:
        pass


def test_offsets():
    testdata = "this\nhas\nsome\nlines\nin it"
    cache = LineCache(len(testdata), None, 0, testdata)
    assert cache.offsets[-1] == cache.total_bytes
    assert testdata[cache.offsets[1]:cache.offsets[2]] == "has\n"


def test_cache_get_data():
    testdata = """aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
    bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
    ccccccccccccccccccccccccccccccc
    ddddddddddddddddddddddddddddddd"""
    peer = MockPeer(testdata)
    first_chunk = testdata[:32]

    cache = LineCache(len(testdata), peer, 0, first_chunk)
    assert len(cache.raw_lines) == 1
    assert cache.raw_lines[0][-1] == '\n'
    assert cache.has_missing
    cache.get_data(peer, 64)
    assert len(cache.raw_lines) == 2
    assert cache.offsets[-1] == 64
    assert cache.raw_lines[1][10] == 'b'

    # no newlines:
    peer.max_size = 8
    cache = LineCache(len(testdata), peer, 0, testdata[:8])
    cache.get_data(peer, 16)
    assert len(cache.raw_lines) == 1


def test_get_data_on_missing_line():
    testdata = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\nu\nv\nw\nx\ny\nz"
    peer = MockPeer(testdata)
    cache = LineCache(len(testdata), peer, 0, testdata[:8])
    assert len(cache.raw_lines) == 4
    assert cache[14] == 'o'


def test_update():
    testdata = "this\nhas\nsome\nlines\nin it"
    cache = LineCache(len(testdata), None, 0, testdata)
    cache.apply_update(None, 'author', 0, 4, 4, 3, 'insert', 'tle')
    assert cache.raw_lines[0] == "thistle\n"
    assert cache.offsets[1] == len(cache.raw_lines[0])
    assert cache.offsets[-1] == len(testdata) + 3
    assert len(cache.offsets) == len(cache.raw_lines) + 1
    assert len(cache.raw_lines) == 5

    cache.apply_update(None, 'author', 1, 10, 11, 5, 'insert', 'ha\noh')
    assert len(cache.raw_lines) == 6
    assert cache.raw_lines[1] == "haha\n"
    assert cache.raw_lines[2] == "oh\n"


def test_empty_buff():
    testdata = b""
    cache = LineCache(0, None, 0, testdata)
    assert len(cache.raw_lines) == 1
    assert len(cache.offsets) == 2
    cache.apply_update(None, 'author', 1, 0, 0, 1, 'insert', 'q')
    assert len(cache.offsets) == 2
    cache.apply_update(None, 'author', 2, 1, 1, 1, 'insert', '\n')
    assert len(cache.offsets) == 2
    cache.apply_update(None, 'author', 3, 2, 2, 1, 'insert', 'z')
    assert len(cache.raw_lines) == 2
    assert cache[0] == 'q'
    assert len(cache.offsets) == 3


def test_delete_all():
    testdata = "this\nhas\nsome\nlines\nin it"
    cache = LineCache(len(testdata), None, 0, testdata)
    cache.apply_update(None, 'author', 1, 0, len(testdata), 0, 'breakpoint', '')
    assert cache.total_bytes == 0
    assert sum(map(len, cache.raw_lines)) == 0


def test_linecol():
    testdata = "abc\ndef\nghi\njkl\nmno\n"
    cache = LineCache(len(testdata), None, 0, testdata)
    assert cache.linecol_for_offset(0) == (0, 0)
    assert cache.linecol_for_offset(1) == (0, 1)
    assert cache.linecol_for_offset(6) == (1, 2)
    try:
        assert cache.linecol_for_offset(100)
        assert False
    except IndexError:
        pass


def test_prev_word():
    testdata = "this is a single line\n"
    cache = LineCache(len(testdata), None, 0, testdata)
    assert cache.previous_word(3) == "thi"
    assert cache.previous_word(8) == "is"
    assert cache.previous_word(7) == "is"
