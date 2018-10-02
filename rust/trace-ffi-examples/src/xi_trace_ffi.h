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

#pragma once

#include <stdbool.h>
#include <stdlib.h>

struct xi_trace_block_t;
typedef struct xi_trace_block_t xi_trace_block_t;

typedef void* (*xi_trace_allocator_t)(size_t);

extern size_t xi_trace_samples_len(void);
extern void xi_trace_disable(void);
extern void xi_trace_enable(void);
extern bool xi_trace_is_enabled(void);
extern void xi_trace(const char *name, const char ** categories);
extern xi_trace_block_t* xi_trace_block_begin(const char *name, const char ** categories);
extern void xi_trace_block_end(xi_trace_block_t* trace_block);
extern void* xi_trace_serialize_to_mem(xi_trace_allocator_t allocator, size_t *len);

