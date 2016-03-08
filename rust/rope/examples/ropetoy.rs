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

//! Toy app for experimenting with ropes

extern crate dimer_rope;

use dimer_rope::Rope;

fn main() {
    let mut a = Rope::from_str("hello.");
    a = a.replace_str(5, 6, "!");
    for _ in 0..1000000 {
        let l = a.size();
        //a = a.replace_str(l, l, &(i.to_string() + "\n"));
        a = a.replace_str(l, l, "aaaaaaa");
    }
    let l = a.size();
    a = a.replace_str(1000, l, "");
    //a = a.subrange(0, 1000);
    println!("{:?}", a);
}
