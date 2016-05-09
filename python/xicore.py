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
import struct
import json

def sendraw(buf):
    sys.stdout.write(struct.pack("<q", len(buf)))
    sys.stdout.write(buf)
    sys.stdout.flush()

def send(obj):
    sendraw(json.dumps(obj))

def mainloop():
    text = ''
    while True:
        sizebuf = sys.stdin.read(8)
        if len(sizebuf) == 0:
            return
        (size,) = struct.unpack("<q", sizebuf)
        cmd, arg = json.loads(sys.stdin.read(size))
        print >> sys.stderr, cmd, arg
        if cmd == 'key':
            chars = arg['chars']
            if chars == u'\x7f':
                if len(text):
                    text = text[:-1]
            else:
                text += chars
            send(['settext', text])

mainloop()
