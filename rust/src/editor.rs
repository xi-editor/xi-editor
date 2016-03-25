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

use ::send;

pub struct Editor {
	text: Rope
}

fn settext(text: &str) {
    let value = ArrayBuilder::new()
        .push("settext")
        .push(text)
        .unwrap();
    send(&value);
}

impl Editor {
	pub fn new() -> Editor {
		Editor {
			text: Rope::from("")
		}
	}

	pub fn do_cmd(&mut self, cmd: &str, args: &Value) {
		if cmd == "key" {
           	if let Some(args) = args.as_object() {
	            let chars = args.get("chars").unwrap().as_string().unwrap();
                if chars == "\x7f" {
                    // TODO: implement complex emoji logic
                    if let Some(bsp_pos) = self.text.prev_codepoint_offset(self.text.len()) {
                        self.text = self.text.clone().slice(0, bsp_pos);
                    } else {
                        return;
                    }
                } else {
                    self.text.push_str(chars);
                }
                settext(&String::from(&self.text));
            }
        }
	}
}
