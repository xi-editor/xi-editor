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

import bisect
from xi_plugin.cache import LineCache

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
