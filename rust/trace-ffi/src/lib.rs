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

extern crate libc;
extern crate xi_trace;
extern crate xi_trace_dump;

use libc::{c_char, c_void, size_t};
use std::ffi::CStr;

#[derive(Debug)]
enum ConversionError {
    NullPointer,
    Encoding(std::str::Utf8Error),
}

pub type MallocSignature = extern fn(size: size_t) -> *mut c_void;

impl std::fmt::Display for ConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {
            ConversionError::NullPointer => write!(f, "NullPointer"),
            ConversionError::Encoding(err) => write!(f, "Encoding({})", err)
        }
    }
}

fn c_from_str(c_str: *const c_char) -> Result<String, ConversionError> {
    if c_str.is_null() {
        Err(ConversionError::NullPointer)
    } else {
        unsafe {
            CStr::from_ptr(c_str).to_str()
                .map_err(|e| ConversionError::Encoding(e))
                .map(|s| s.to_string())
        }
    }
}

fn c_from_categories(c_categories: *const *const c_char)
                     -> Result<Vec<String>, ConversionError>
{
    if c_categories.is_null() {
        return Ok(Vec::new())
    }

    unsafe {
        let mut categories = Vec::new();
        let mut c_categories_iter = c_categories;
        while !(*c_categories_iter).is_null() {
            let category = c_from_str(*c_categories);
            if category.is_err() {
                return Err(category.unwrap_err());
            }
            categories.push(category.unwrap());
            c_categories_iter = c_categories_iter.offset(1);
        }
        return Ok(categories);
    }
}

/// Disable tracing & discard all sample data.  See `xi_trace::disable_tracing`.
/// All samples attempting to record after this function call will also be
/// discarded.  The default is for tracing to be disabled.
#[no_mangle]
pub unsafe extern "C" fn xi_trace_disable() {
    xi_trace::disable_tracing();
}

/// Enable tracing with the default configuration.  See
/// `xi_trace::enable_tracing`. Default is disabled.
#[no_mangle]
pub unsafe extern "C" fn xi_trace_enable() {
    xi_trace::enable_tracing();
}

#[no_mangle]
pub unsafe extern "C" fn xi_trace_is_enabled() -> bool {
    xi_trace::is_enabled()
}

/// C function for creating an instantaneous sample.  See `xi_trace::trace`.
/// If any of the arguments fail to parse (e.g. malformed UTF-8 or null pointer)
/// this function is a no-op.
///
/// # Performance
/// This is heavier-weight than invoking `xi_trace::trace` directly due to the
/// need to copy all values passed in by the caller into Rust objects.
///
/// # Arguments
///
/// `c_name` - A null-terminated UTF-8 string.
/// `c_categories` - A null-terminated array of null-terminated UTF-8 strings.
///
/// # Examples
///
/// ```text
/// xi_trace("something", (const char *[]){"ffi", "rpc"});
/// ```
#[no_mangle]
pub extern "C" fn xi_trace(c_name: *const c_char,
                           c_categories: *const *const c_char)
{
    if !xi_trace::is_enabled() {
        return;
    }

    let name = c_from_str(c_name);
    let categories = c_from_categories(c_categories);

    if name.is_err() || categories.is_err() {
        if name.is_err() {
            eprintln!("Couldn't convert name: {}", name.unwrap_err());
        }

        if categories.is_err() {
            eprintln!("Couldn't convert categories: {}",
                      categories.unwrap_err());
        }
        return;
    }

    xi_trace::trace(name.unwrap(), categories.unwrap());
}

/// Creates a sample for a block of code.  The returned opaque value should be
/// passed to trace_block_end when the section of code to be measured completes.
/// Failure to call trace_block_end will result in a memory leak (maybe even if
/// tracing is disabled).  The returned value is opaque.
///
/// # Performance
///
/// See `trace`
///
/// # Arguments
/// See `trace`
///
/// # Examples
/// ```text
/// extern void* xi_trace_block_begin(const char *name, const char *categories[]);
/// extern void xi_trace_block_end(void* trace_block);
///
/// void *trace_block = xi_trace_block_begin("something", (const char *[]){"ffi", "rpc"});
/// xi_trace_block_end(trace_block);
/// ```
#[no_mangle]
pub extern "C" fn xi_trace_block_begin(c_name: *const c_char,
                                       c_categories: *const *const c_char)
    -> *mut xi_trace::SampleGuard<'static> {
    if !xi_trace::is_enabled() {
        return std::ptr::null_mut();
    }

    let name = c_from_str(c_name);
    let categories = c_from_categories(c_categories);

    if name.is_err() || categories.is_err() {
        if name.is_err() {
            eprintln!("Couldn't convert name: {}", name.unwrap_err());
        }

        if categories.is_err() {
            eprintln!("Couldn't convert categories: {}",
                      categories.unwrap_err());
        }
        return std::ptr::null_mut();
    }

    let result = Box::new(xi_trace::trace_block(
            name.unwrap(), categories.unwrap()));
    return Box::into_raw(result);
}

/// Finalizes the block that was started via `trace_block_begin`.  See
/// `trace_block_begin` for more info.
#[no_mangle]
pub extern "C" fn xi_trace_block_end(
    trace_block: *mut xi_trace::SampleGuard<'static>) {
    if trace_block.is_null() {
        return;
    }

    unsafe {
        Box::from_raw(trace_block);
    }
}

/// Returns the numbers of samples recorded so far.
#[no_mangle]
pub extern "C" fn xi_trace_samples_len() -> size_t {
    xi_trace::samples_len()
}

/// Serializes all the samples captured so far into a format for IPC
/// transmission.
///
/// # Arguments
/// `malloc` - The pointer to the allocator to use for the buffer.
/// `len` - The allocated length is written here on successful serialization.
/// On failure this will have 0.
///
/// # Returns
/// A pointer to the allocated buffer containing the serialized data.  On
/// failure this will return `null`.
///
/// # Examples
/// ```text
/// size_t len;
/// void *serialized = xi_trace_serialize_to_mem(malloc, &len);
/// if (!serialized) {
///   // handle error
/// } else {
///   // send serialized over IPC
/// }
/// ```
// TODO(vlovich): maybe we need a base64 version to make it safe to transmit
// via JSON?
#[no_mangle]
pub extern "C" fn xi_trace_serialize_to_mem(malloc: MallocSignature,
                                            len: *mut usize)
    -> *mut c_void
{
    assert!(!(malloc as *const MallocSignature).is_null(), "No malloc function provided");
    assert!(!len.is_null(), "No pointer to save allocated length provided");

    unsafe {
        *len = 0;
    }

    let samples = xi_trace::samples_cloned_sorted();
    let serialized_len_u64 = xi_trace_dump::ipc::serialized_size(&samples);
    if serialized_len_u64 > std::usize::MAX as u64 {
        eprintln!("Serializing samples won't fit in memory, {} bytes",
                  serialized_len_u64);
        return std::ptr::null_mut();
    }

    let serialized_len = serialized_len_u64 as usize;

    let serialized = malloc(serialized_len) as *mut u8;

    if serialized.is_null() {
        eprintln!("Failed to allocate {} bytes", serialized_len_u64);
        return std::ptr::null_mut();
    }

    let mut serialized_slice = unsafe {
        std::slice::from_raw_parts_mut(serialized, serialized_len as usize)
    };

    let err = xi_trace_dump::ipc::serialize_to_stream(&samples, &mut serialized_slice);
    if err.is_err() {
        eprintln!("Failed to serialize samples: {:?}", err.unwrap_err());
        return std::ptr::null_mut();
    }

    unsafe {
        *len = serialized_len;
    }
    serialized as *mut c_void
}

