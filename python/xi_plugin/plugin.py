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

import sys

from . import edit
from .rpc import RpcPeer
from .cache import LineCache

# max bytes of buffer per request
MAX_FETCH_SIZE = 1024*1024


class PluginPeer(RpcPeer):
    """A proxy object which wraps RPC methods implemented in xi-core."""

    def update_spans(self, start, length, spans, rev):
        self.send_rpc('update_spans', {'start': start,
                                       'len': length,
                                       'spans': spans,
                                       'rev': rev})

    def add_scopes(self, scopes):
        self.send_rpc('add_scopes', {'scopes': scopes})

    def get_data(self, from_offset, rev, max_size=MAX_FETCH_SIZE):
        return self.send_rpc_sync('get_data', {'offset': from_offset,
                                               "max_size": max_size,
                                               "rev": rev})



class Plugin(object):
    """Base class for python plugins.

    RPC requests are converted to methods with the same same signature.
    A cache of the open buffer is available at `self.lines`.
    """
    def __init__(self):
        self.cache = None
        self.identifier = type(self).__name__
        self.path = None,
        self.syntax = "plaintext"

    def __call__(self, method, params, peer):
        params = params or {}
        # intercept these to do bookkeeping, so subclasses don't have to call super
        self.__last_method = method

        if method == "initialize":
            params = params['buffer_info']
            self._initialize(peer, **params)
        if method == "update":
            self._update(peer, params)
        if method == "shutdown":
            peer.done = True

        return getattr(self, method, self.noop)(peer, **params)

    def print_err(self, err):
        print("PLUGIN.PY {}>>> {}".format(self.identifier, err), file=sys.stderr)
        sys.stderr.flush()

    def new_edit(self, rev, edit_range, new_text,
                  priority=edit.EDIT_PRIORITY_NORMAL, after_cursor=False):
        return edit.Edit(rev, edit_range, new_text, self.identifier, priority, after_cursor)

    def ping(self, peer, **kwargs):
        self.print_err("ping")

    def _initialize(self, peer, rev, buf_size, nb_lines, syntax, path=None):
        # fetch an initial chunk (this isn't great: we don't know
        # where the cursor is)
        self.lines = LineCache(buf_size, peer, rev)
        self.path = path
        self.syntax = syntax

    def _update(self, peer, params):
        self.lines.apply_update(peer, **params)

    def noop(self, *args, **kwargs):
        """Default responder for unimplemented RPC methods."""
        return self.print_err("{} not implemented".format(self.__last_method))


def start_plugin(plugin):
    """Opens an RPC connection and runs indefinitely."""
    # this doesn't need to be exported
    peer = PluginPeer(plugin)
    peer.mainloop()
    plugin.print_err("ended main loop")
