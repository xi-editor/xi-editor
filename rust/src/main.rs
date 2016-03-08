/// Copyright 2016 Google Inc. All rights reserved.
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

extern crate serde_json;

use serde_json::Value;

fn main() {
    let data: Value = serde_json::from_str("{\"a\": 42}").unwrap();
    println!("data: {:?}", data.as_object().unwrap().get("a").unwrap());
    println!("data as string: {}", serde_json::to_string(&data).unwrap());
}
