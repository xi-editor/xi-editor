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

#while True:
#    sys.stderr.write('[' + sys.stdin.read(1) + ']\n')

def mainloop():
    sys.stderr.write("plugin start\n")
    while True:
        line = sys.stdin.readline()
        sys.stderr.write('got line, size=%d\n' % len(line))
        if len(line) == 0:
            break
        data = json.loads(line)
        method = data['method']
        params = data['params']
        sys.stderr.write(method + ": " + json.dumps(params) + "\n")
    sys.stderr.write("exit main loop\n")

mainloop()
