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

use serde_json::Value;

use xi_rope::rope::RopeDelta;
use xi_rope::delta::{DeltaElement, Delta};
use xi_rpc::RpcPeer;
use xi_core::{ViewIdentifier, PluginPid};

use plugin_base::{Error};

const CHUNK_SIZE: usize = 1024 * 1024;

/// A byte-oriented cache tracking a remote document.
pub trait RawCache {
    /// Get a slice of the document, containing at least the given interval
    /// plus at least one more codepoint (unless at EOF).
    fn get_slice(&mut self, start: usize, end: usize) -> Result<&str, Error>;
    /// Updates the cache by applying this delta'.
    fn apply_delta(&mut self, delta: &RopeDelta);
    /// Flushes any state held by this cache.
    fn clear(&mut self);
}

/// A simple cache, holding a single contiguous chunk of the document.
#[derive(Debug, Clone, Default)]
pub struct ChunkCache<DS> {
    /// The position of this chunk relative to the tracked document.
    offset: usize,
    contents: String,
    /// The total size of the tracked document.
    buf_size: usize,
    // only optional so we can get `Default`, and play nicely with
    // existing state cache
    datasource: Option<DS>,
}

/// Abstracts getting data from the peer. This only exists so we can mock it in tests.
pub trait DataSource {
    fn get_data(&self, offset: usize, max_size: usize) -> Result<String, Error>;
}

/// Single-purpose handle to the RPC channel.
///
/// This lets us avoid having to pass a handle to the peer every time
/// we do something that might require a fetch.
pub struct RemoteDataSource {
    peer: RpcPeer,
    plugin_id: PluginPid,
    view_id: ViewIdentifier,
    //TODO: unclear if this should be here or in ChunkCache
    rev: u64,
}

impl<DS: DataSource> RawCache for ChunkCache<DS> {
    fn get_slice(&mut self, start: usize, end: usize) -> Result<&str, Error>
    {
        loop {
            let chunk_start = self.offset;
            let chunk_end = chunk_start + self.contents.len();
            if start >= chunk_start && (start < chunk_end || chunk_end == self.buf_size) {
                // At least the first codepoint at start is in the chunk.
                if end < chunk_end || chunk_end == self.buf_size {
                    return Ok(&self.contents[start - chunk_start ..]);
                }
                let new_chunk = self.datasource.as_ref()
                    .expect("datasource must be set")
                    .get_data(chunk_end, CHUNK_SIZE)?;
                if start == chunk_start {
                    self.contents.push_str(&new_chunk);
                } else {
                    self.offset = start;
                    self.contents = [&self.contents[start - chunk_start ..],
                                     &new_chunk].concat();
                }
            } else {
                // TODO: if chunk_start < start + CHUNK_SIZE, could fetch smaller
                // chunk and concat; probably not a major savings in practice.
                self.contents = self.datasource.as_ref()
                    .expect("datasource must be set")
                    .get_data(start, CHUNK_SIZE)?;
                self.offset = start;
            }
        }
    }

    /// Updates the chunk to reflect changes in this delta.
    fn apply_delta(&mut self, delta: &RopeDelta) {
        if self.offset == 0 && self.contents.len() == 0 {
            return
        }

        let chunk_start = self.offset;
        let chunk_end = chunk_start + self.contents.len();
        let mut new_state = String::with_capacity(self.contents.len());
        let mut prev_copy_end = 0;
        let mut del_before: usize = 0;
        let mut ins_before: usize = 0;

        for op in delta.els.as_slice() {
            match op {
                &DeltaElement::Copy(start, end) => {
                    if start < chunk_start {
                        del_before += start - prev_copy_end;
                        if end >= chunk_start {
                            let cp_end = (end - chunk_start).min(self.contents.len());
                            new_state.push_str(&self.contents[0..cp_end]);
                        }
                    } else if start <= chunk_end {
                        if prev_copy_end < chunk_start {
                            del_before += chunk_start - prev_copy_end;
                        }
                        let cp_start = start - chunk_start;
                        let cp_end = (end - chunk_start).min(self.contents.len());
                        new_state.push_str(&self.contents[cp_start .. cp_end]);
                    }
                    prev_copy_end = end;
                }
                &DeltaElement::Insert(ref s) => {
                    if prev_copy_end < chunk_start {
                        ins_before += s.len();
                    } else if prev_copy_end <= chunk_end {
                        let s: String = s.into();
                        new_state.push_str(&s);
                    }
                }
            }
        }

        self.buf_size = delta.new_document_len();
        self.offset += ins_before;
        self.offset -= del_before;
        self.contents = new_state;
    }

    fn clear(&mut self) {
        self.contents.clear();
        self.offset = 0;
    }
}

impl DataSource for RemoteDataSource {
    fn get_data(&self, offset: usize, max_size: usize) -> Result<String, Error> {
        let result = self.peer.send_rpc_request("get_data", &json!({
            "plugin_id": self.plugin_id,
            "view_id": self.view_id,
            "offset": offset,
            "max_size": max_size,
            "rev": self.rev,
        }));

        match result {
            Ok(Value::String(s)) => Ok(s),
            Ok(_) => Err(Error::WrongReturnType),
            Err(err) => Err(Error::RpcError(err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xi_rope::interval::Interval;

    struct MockDataSource(String);

    impl DataSource for MockDataSource {
        fn get_data(&self, offset: usize, max_size: usize) -> Result<String, Error> {
            // not the right error, but okay for this
            let end = self.0.len().min(offset+max_size);
            if offset > self.0.len() || !self.0.is_char_boundary(offset) || !self.0.is_char_boundary(end) {
                Err(Error::WrongReturnType)
            } else {
                Ok( unsafe{ self.0.slice_unchecked(offset, end).into() } )
            }
        }
    }

    #[test]
    fn simple_chunk() {
        let datasource = MockDataSource("oh".into());
        let mut c = ChunkCache {
            offset: 0,
            contents: "oh".into(),
            buf_size: 2,
            datasource: Some(datasource)
        };
        let d = Delta::simple_edit(Interval::new_closed_open(0, 0), "yay".into(), c.contents.len());
        c.apply_delta(&d);
        assert_eq!(&c.contents, "yayoh");
        assert_eq!(c.offset, 0);

        let d = Delta::simple_edit(Interval::new_closed_open(0, 0), "ahh".into(), c.contents.len());
        c.apply_delta(&d);

        assert_eq!(&c.contents, "ahhyayoh");
        assert_eq!(c.offset, 0);

        let d = Delta::simple_edit(Interval::new_closed_open(2, 3), "oops".into(), c.contents.len());
        assert_eq!(d.els.len(), 3);
        c.apply_delta(&d);

        assert_eq!(&c.contents, "ahoopsyayoh");
        assert_eq!(c.offset, 0);

        let d = Delta::simple_edit(Interval::new_closed_open(9, 9), "fin".into(), c.contents.len());
        c.apply_delta(&d);

        assert_eq!(&c.contents, "ahoopsyayfinoh");
        assert_eq!(c.offset, 0);
    }


    #[test]
    fn offset_chunk() {
        let datasource = MockDataSource("this is a tenchars!!".into());
        let mut c = ChunkCache {
            offset: 10,
            contents: "tenchars!!".into(),
            datasource: Some(datasource),
            buf_size: 20
        };

        let d = Delta::simple_edit(Interval::new_closed_open(0, 0), "yay".into(),
                                   c.offset + c.contents.len());
        c.apply_delta(&d);
        assert_eq!(c.offset, 13);
        assert_eq!(&c.contents, "tenchars!!");

        let d = Delta::simple_edit(Interval::new_closed_open(16, 0), "t".into(),
                                   c.offset + c.contents.len());
        c.apply_delta(&d);
        assert_eq!(c.offset, 13);
        assert_eq!(&c.contents, "tentchars!!");

        let d = Delta::simple_edit(Interval::new_closed_open(5, 15), "stu".into(),
                                   c.offset + c.contents.len());
        c.apply_delta(&d);
        assert_eq!(c.offset, 8);
        assert_eq!(&c.contents, "ntchars!!");

        // some edit off the end of the chunk
        let d = Delta::simple_edit(Interval::new_closed_open(50, 50), "hmm".into(),
                                   50);
        c.apply_delta(&d);
        assert_eq!(c.offset, 8);
        assert_eq!(&c.contents, "ntchars!!");

        let d = Delta::simple_edit(Interval::new_closed_open(15, 17), "???".into(),
                                   c.offset + c.contents.len());
        c.apply_delta(&d);
        assert_eq!(c.offset, 8);
        assert_eq!(&c.contents, "ntchars???");
    }
}
