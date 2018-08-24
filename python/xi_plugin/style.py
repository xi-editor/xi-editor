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

"""Provides convenience methods for styling text."""

BOLD = 1
UNDERLINE = 2
ITALIC = 4


def color_for_rgba_float(red, green, blue, alpha=1):
    if any(map(lambda x: x < 0 or x > 1, (red, green, blue, alpha))):
        raise ValueError("Values must be in the range 0..1 (inclusive)")
    red, green, blue, alpha = map(lambda c: int(0xFF * c), (red, green, blue, alpha))
    return (alpha << 24) | (red << 16) | (green << 8) | blue
