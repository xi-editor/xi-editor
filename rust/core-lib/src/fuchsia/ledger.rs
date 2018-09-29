// Copyright 2017 The xi-editor Authors.
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

//! Utility functions to make it easier to work with Ledger in Rust
// TODO merge with equivalent module in fuchsia/rust_ledger_example into a library?

use apps_ledger_services_public::*;
use fidl::Error;
use fuchsia::read_entire_vmo;
use magenta::{self, Vmo};
use sha2::{Digest, Sha256};

// Rust emits a warning if matched-on constants aren't all-caps
pub const OK: Status = Status_Ok;
pub const KEY_NOT_FOUND: Status = Status_KeyNotFound;
pub const NEEDS_FETCH: Status = Status_NeedsFetch;
pub const RESULT_COMPLETED: ResultState = ResultState_Completed;

pub fn ledger_crash_callback(res: Result<Status, Error>) {
    let status = res.expect("ledger call failed to respond with a status");
    assert_eq!(status, Status_Ok, "ledger call failed");
}

#[derive(Debug)]
pub enum ValueError {
    NeedsFetch,
    LedgerFail(Status),
    Vmo(magenta::Status),
}

/// Convert the low level result of getting a key from the ledger into a
/// higher level Rust representation.
pub fn value_result(res: (Status, Option<Vmo>)) -> Result<Option<Vec<u8>>, ValueError> {
    match res {
        (OK, Some(vmo)) => {
            let buffer = read_entire_vmo(&vmo).map_err(ValueError::Vmo)?;
            Ok(Some(buffer))
        }
        (KEY_NOT_FOUND, _) => Ok(None),
        (NEEDS_FETCH, _) => Err(ValueError::NeedsFetch),
        (status, _) => Err(ValueError::LedgerFail(status)),
    }
}

/// Ledger page ids are exactly 16 bytes, so we need a way of determining
/// a unique 16 byte ID that won't collide based on some data we have
pub fn gen_page_id(input_data: &[u8]) -> [u8; 16] {
    let mut hasher = Sha256::default();
    hasher.input(input_data);
    let full_hash = hasher.result();
    let full_slice = full_hash.as_slice();

    // magic incantation to get the first 16 bytes of the hash
    let mut arr: [u8; 16] = Default::default();
    arr.as_mut().clone_from_slice(&full_slice[0..16]);
    arr
}
