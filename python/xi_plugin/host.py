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

from .cache import LineCache
from .plugin import Plugin, GlobalPlugin
from .rpc import RpcPeer
from .view import View


# max bytes of buffer per request
MAX_FETCH_SIZE = 1024*1024


class PluginPeer(RpcPeer):
    """A proxy object which wraps RPC methods implemented in xi-core."""
    _plugin_pid = None

    @property
    def plugin_pid(self):
        assert self._plugin_pid, 'plugin_pid must be set before sending rpcs'
        return self._plugin_pid

    def edit(self, view_id, edit):
        self.send_rpc('edit', {'view_id': view_id,
                               'plugin_id': self.plugin_pid,
                               'edit': edit.to_dict()})

    def update_spans(self, view_id, start, length, spans, rev):
        self.send_rpc('update_spans', {'view_id': view_id,
                                       'plugin_id': self.plugin_pid,
                                       'start': start,
                                       'len': length,
                                       'spans': spans,
                                       'rev': rev})

    def add_scopes(self, view_id, scopes):
        self.send_rpc('add_scopes', {'view_id': view_id,
                                     'plugin_id': self.plugin_pid,
                                     'scopes': scopes})

    def get_data(self, view_id, from_offset, rev, max_size=MAX_FETCH_SIZE):
        return self.send_rpc_sync('get_data', {'view_id': view_id,
                                               'plugin_id': self.plugin_pid,
                                               'offset': from_offset,
                                               'max_size': max_size,
                                               'rev': rev})

    def get_selections(self, view_id):
        return self.send_rpc_sync('get_selections', {'view_id': view_id,
                                                     'plugin_id': self.plugin_pid,
                                                     })


class PluginHost(object):
    """Handles raw RPC calls, updating state and calling plugin methods
    as appropriate."""

    def __init__(self, plugin):
        self.plugin = plugin
        self.views = dict()

    def initialize(self, peer, plugin_pid, buffer_info):
        peer._plugin_pid = plugin_pid
        self._initialize_buffers(peer, buffer_info)

        if isinstance(self.plugin, GlobalPlugin):
            first_views = [b["views"][0] for b in buffer_info]
            self.plugin.initialize([self.views[v] for v in first_views])

        else:
            assert len(buffer_info) == 1
            first_view = buffer_info[0]["views"][0]
            self.plugin.initialize(self.views[first_view])

    def new_buffer(self, peer, buffer_info):
        assert isinstance(self.plugin, GlobalPlugin)
        assert len(buffer_info) == 1
        self._initialize_buffers(peer, buffer_info)
        first_view = buffer_info[0]["views"][0]
        self.plugin.new_buffer(self.views[first_view])

    def did_save(self, peer, view_id, path):
        """Notification that a buffer was saved."""
        view = self.views[view_id]
        old_path = view.lines.path
        view.lines.path = path
        self.plugin.did_save(view, old_path)

    def did_close(self, peer, view_id):
        """Notification that a view was closed."""
        assert isinstance(self.plugin, GlobalPlugin)
        del self.views[view_id]
        self.plugin.did_close(view_id)

    def update(self, peer, **params):
        """Request sent when an update (edit) has occurred in a view."""
        view_id = params.pop("view_id")
        view = self.views[view_id]
        view.lines.apply_update(peer, **params)
        return self.plugin.update(view, **params)

    def ping(self, peer, **params):
        pass

    def shutdown(self, peer, **params):
        peer.done = True
        self.plugin.shutdown()

    def custom_command(self, peer, method, params):
        '''Custom command provided by the plugin.'''
        if params.get('view'):
            view = self.views.get(params['view'])
            params['view'] = view
        return getattr(self.plugin, method)(**params)

    def _initialize_buffers(self, peer, buffer_info):
        for buf in buffer_info:
            lines = LineCache(peer, **buf)
            for view_id in buf['views']:
                self.views[view_id] = View(view_id, lines)


def start_plugin(plugin):
    """Opens an RPC connection and runs indefinitely."""
    host = PluginHost(plugin)
    peer = PluginPeer(host)
    peer.mainloop()
    plugin.print_err("ended main loop")
