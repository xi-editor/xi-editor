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

#include "../src/api.h"
#include "../src/xi_trace_ffi.h"

API void bench_trace_no_categories_iter() {
  xi_trace("no categories", (const char *[]){NULL});
}

API void bench_trace_one_category_iter() {
  xi_trace("no categories", (const char *[]){"bench", NULL});
}

API void bench_trace_two_categories_iter() {
  xi_trace("no categories", (const char *[]){"bench", "rpc", NULL});
}

API void bench_trace_block_no_categories_iter() {
  xi_trace_block_t *block = xi_trace_block_begin("no categories", (const char *[]){NULL});
  xi_trace_block_end(block);
}

