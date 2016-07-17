#!/usr/bin/env python

# Copyright 2016 Google Inc. All rights reserved.
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
import json
from collections import deque

# This is a simple RPC peer with some limitations. It assumes a single-threaded
# execution model, and only allows one outgoing RPC at a time. Incoming RPC's
# are queued.
#
# Also, in the current implementation, errors are not handled.
class RpcPeer(object):
    def __init__(self, handler, stdin = None, stdout = None):
        if stdin is None:
            stdin = sys.stdin
        if stdout is None:
            stdout = sys.stdout
        self.handler = handler
        self.stdin = stdin
        self.stdout = stdout
        self.pending = deque()
        self.id_counter = 0
        self.done = False
    def mainloop(self, waiting_for = None):
        while not self.done:
            line = self.stdin.readline()
            if len(line) == 0:
                self.done = True
                return None
            data = json.loads(line)
            if 'id' in data:  # 'id' is unique required field in response objects
                if data['id'] != waiting_for:
                    raise Exception('waiting for ' + str(waiting_for) + ' got ' + str(data['id']))
                return data['result']
            self.pending.append(data)
            if waiting_for is None:
                while self.has_pending():
                    self.handle(self.pending.popleft())
    def handle(self, data):
        req_id = data.get('id', None)
        method = data['method']
        params = data['params']
        result = self.handler(method, params, self)
        if result is not None:
            if req_id is None:
                raise Exception('unexpected return value on method ' + method)
            resp = {'result': result, 'id': req_id}
            self.send(resp)
        elif req_id is not None:
            raise Exception('expected return value on method ' + method + ' id ' + str(id))
    def send(self, data):
        self.stdout.write(json.dumps(data))
        self.stdout.write('\n')
        self.stdout.flush()
    def send_rpc(self, method, params, req_id = None):
        req = {'method': method, 'params': params}
        if req_id is not None:
            req['id'] = req_id
        self.send(req)
    def send_rpc_sync(self, method, params):
        req_id = self.id_counter
        self.id_counter += 1
        self.send_rpc(method, params, req_id)
        return self.mainloop(waiting_for = req_id)
    def has_pending(self):
        return len(self.pending) != 0

class PluginPeer(RpcPeer):
    def n_lines(self):
        return self.send_rpc_sync('n_lines', [])
    def get_line(self, i):
        return self.send_rpc_sync('get_line', {'line': i})
    def set_line_fg_spans(self, i, spans):
        self.send_rpc('set_line_fg_spans', {'line': i, 'spans': spans})

def handler(method, params, peer):
    if method == 'ping':
        sys.stderr.write('ping\n')
    elif method == 'ping_from_editor':
        sys.stderr.write('ping_from_editor ' + json.dumps(params) + '\n')
        n_lines = peer.n_lines()
        sys.stderr.write('got n_lines = %d\n' % n_lines)
        needle = 'fizz'
        for i in range(n_lines):
            line = peer.get_line(i)
            spans = []
            j = 0
            while True:
                j = line.find(needle, j)
                if j == -1:
                    break
                spans.append({'start': j, 'end': j + len(needle), 'fg': 0xffc00000})
                j += len(needle)
            peer.set_line_fg_spans(i, spans)
            sys.stderr.write('%d: %s\n' % (i, line))

def main():
    peer = PluginPeer(handler)
    peer.mainloop()

    sys.stderr.write("exit main loop\n")

main()
