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

use multiset::{SubsetBuilder, Subset};
use delta::{Delta, self};
use rope::{Rope, RopeInfo};
use interval::Interval;

/// Creates a `Subset` of `s` by scanning through `substr` and finding which
/// characters of `s` are missing from it in order. Returns a `Subset` which
/// when deleted from `s` yields `substr`.
pub fn find_deletions(substr: &str, s: &str) -> Subset {
    let mut sb = SubsetBuilder::new();
    let mut j = 0;
    for i in 0..s.len() {
        if j < substr.len() && substr.as_bytes()[j] == s.as_bytes()[i] {
            j += 1;
        } else {
            sb.add_range(i, i + 1, 1);
        }
    }
    sb.pad_to_len(s.len());
    sb.build()
}

impl Delta<RopeInfo> {
    pub fn apply_to_string(&self, s: &str) -> String {
        String::from(self.apply(&Rope::from(s)))
    }
}

impl PartialEq for Rope {
    fn eq(&self, other: &Rope) -> bool {
        String::from(self) == String::from(other)
    }
}

pub fn parse_subset(s: &str) -> Subset {
    let mut sb = SubsetBuilder::new();

    for c in s.chars() {
        if c == '#' {
            sb.push_segment(1,1);
        } else if c == 'e' {
            // do nothing, used for empty subsets
        } else {
            sb.push_segment(1,0);
        }
    }

    sb.build()
}

pub fn parse_subset_list(s: &str) -> Vec<Subset> {
    s.lines().map(|s| s.trim()).filter(|s| !s.is_empty()).map(parse_subset).collect()
}

pub fn debug_subsets(subsets: &[Subset]) {
    for s in subsets {
        println!("{:#?}", s);
    }
}

pub fn parse_delta(s: &str) -> Delta<RopeInfo> {
    let base_len = s.chars().filter(|c| *c == '-' || *c == '!').count();
    let mut b = delta::Builder::new(base_len);

    let mut i = 0;
    for c in s.chars() {
        if c == '-' {
            i += 1;
        } else if c == '!' {
            b.delete(Interval::new_closed_open(i,i+1));
            i += 1;
        } else {
            let inserted = format!("{}", c);
            b.replace(Interval::new_closed_open(i,i), Rope::from(inserted));
        }
    }

    b.build()
}
