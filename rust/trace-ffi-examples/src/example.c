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

#include "api.h"
#include "xi_trace_ffi.h"

#include <stdio.h>
#include <stdlib.h>

API void example_main(void);

void something() {
}

void something_else() {
}

API void example_main(void) {
    fprintf(stderr, "trace enabled = %s\n", xi_trace_is_enabled() ? "yes" : "no");
    xi_trace_enable();
    fprintf(stderr, "trace enabled = %s\n", xi_trace_is_enabled() ? "yes" : "no");

    xi_trace("started", (const char*[]) {"c", "frontend", NULL});
    xi_trace_block_t* total_trace = xi_trace_block_begin("total", (const char*[]) {"c", "frontend", NULL});
    xi_trace_block_t* trace = xi_trace_block_begin("something", (const char*[]) {"c", "frontend", NULL});
    something();
    xi_trace_block_end(trace);
    xi_trace("something_else", (const char*[]) {"c", "frontend", NULL});
    something_else();
    xi_trace_block_end(total_trace);
    fprintf(stderr, "Captured %zu samples\n", xi_trace_samples_len());

    size_t serialized_len;
    void *serialized = xi_trace_serialize_to_mem(malloc, &serialized_len);
    if (!serialized) {
        fprintf(stderr, "Failed to serialize!\n");
        abort();
    }

    fprintf(stderr, "Serialized samples into %zu bytes\n", serialized_len);
}

API void bench_ffi_trace(void) {

}

