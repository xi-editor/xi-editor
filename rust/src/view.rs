// Copyright 2016 Google Inc. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use serde_json::Value;
use serde_json::builder::ArrayBuilder;

use dimer_rope::Rope;

pub struct View {
    pub sel_start: usize,
    pub sel_end: usize,
    first_line: usize  // vertical scroll position
}

impl View {
    pub fn new() -> View {
        View {
            sel_start: 0,
            sel_end: 0,
            first_line: 0
        }
    }

    pub fn render(&self, text: &Rope, nlines: usize) -> Value {
        let sel_start_line = text.line_of_offset(self.sel_start);
        let sel_end_line = if self.sel_start == self.sel_end {
            sel_start_line
        } else {
            text.line_of_offset(self.sel_end)
        };
        let mut result = String::new();
        let mut line_num = self.first_line;
        for l in text.clone().slice(text.offset_of_line(self.first_line), text.len()).lines() {
            if line_num == sel_start_line {
                let sel_start_ix = self.sel_start - text.offset_of_line(line_num);
                result.push_str(&l[..sel_start_ix]);
                result.push('|');
                result.push_str(&l[sel_start_ix..]);
                // TODO: sel_start != sel_end
            } else {
                result.push_str(&l);
            }
            result.push('\n');
            line_num += 1;
            if line_num == self.first_line + nlines {
                break;
            }
        }
        if line_num == sel_end_line {
            result.push('|');
        }
        ArrayBuilder::new()
            .push("settext")
            .push(&result)
            .unwrap()
    }
}
