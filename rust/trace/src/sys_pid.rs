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

#[cfg(all(target_family = "unix", not(target_os = "fuchsia")))]
#[inline]
pub fn current_pid() -> u64 {
    extern "C" {
        fn getpid() -> libc::pid_t;
    }

    unsafe { getpid() as u64 }
}

#[cfg(target_os = "fuchsia")]
pub fn current_pid() -> u64 {
    // TODO: implement for fuchsia (does getpid work?)
    0
}

#[cfg(target_family = "windows")]
#[inline]
pub fn current_pid() -> u64 {
    extern "C" {
        fn GetCurrentProcessId() -> libc::c_ulong;
    }

    unsafe { u64::from(GetCurrentProcessId()) }
}
