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

pub mod ledger;
pub mod sync;

use magenta::{Status, Vmo};

// TODO: move this into magenta-rs?
pub fn read_entire_vmo(vmo: &Vmo) -> Result<Vec<u8>, Status> {
    let size = vmo.get_size()?;
    // TODO: how fishy is this cast to usize?
    let mut buffer: Vec<u8> = Vec::with_capacity(size as usize);
    // TODO: consider using unsafe .set_len() to get uninitialized memory
    for _ in 0..size {
        buffer.push(0);
    }
    let bytes_read = vmo.read(buffer.as_mut_slice(), 0)?;
    buffer.truncate(bytes_read);
    Ok(buffer)
}
