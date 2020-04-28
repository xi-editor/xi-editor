// Copyright 2018 The xi-editor Authors.
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

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[inline]
pub fn current_tid() -> Result<u64, libc::c_int> {
    #[link(name = "pthread")]
    extern "C" {
        fn pthread_threadid_np(thread: libc::pthread_t, thread_id: *mut u64) -> libc::c_int;
    }

    unsafe {
        let mut tid = 0;
        let err = pthread_threadid_np(0, &mut tid);
        match err {
            0 => Ok(tid),
            _ => Err(err),
        }
    }
}

#[cfg(target_os = "fuchsia")]
#[inline]
pub fn current_tid() -> Result<u64, libc::c_int> {
    // TODO: fill in for fuchsia.  This is the native C API but maybe there are
    // rust-specific bindings already.
    /*
    extern {
        fn thrd_get_zx_handle(thread: thrd_t) -> zx_handle_t;
        fn thrd_current() -> thrd_t;
    }

    Ok(thrd_get_zx_handle(thrd_current()) as u64)
    */
    Ok(0)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[inline]
pub fn current_tid() -> Result<u64, libc::c_int> {
    unsafe { Ok(libc::syscall(libc::SYS_gettid) as u64) }
}

// TODO: maybe use https://github.com/alexcrichton/cfg-if to simplify this?
// pthread-based fallback
#[cfg(all(
    target_family = "unix",
    not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "linux",
        target_os = "android",
        target_os = "fuchsia"
    ))
))]
pub fn current_tid() -> Result<u64, libc::c_int> {
    unsafe { Ok(libc::pthread_self() as u64) }
}

#[cfg(target_os = "windows")]
#[inline]
pub fn current_tid() -> Result<u64, libc::c_int> {
    extern "C" {
        fn GetCurrentThreadId() -> libc::c_ulong;
    }

    unsafe { Ok(u64::from(GetCurrentThreadId())) }
}
