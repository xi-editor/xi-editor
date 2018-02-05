// Copyright 2018 Google LLC
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

extern {
    fn example_main();
}

fn main() {
    // NOTE: we can't access xi_trace from here because then we'd end up linking the
    // xi_trace library twice: once via xi_trace_ffi and once directly on xi_trace via Rust
    // which results either in duplicate symbols (static linkage) or 2 instances of the
    // TRACE singleton (dynamic linkage).
    unsafe { example_main(); }
}

