// Copyright 2018 Google Inc. All rights reserved.
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

//! The simplest cache. This should eventually offer line-oriented access
//! to the remote document, and can be used as a building block for more
//! complicated caching schemes.

use memchr::memchr;

use xi_rope::rope::RopeDelta;
use xi_rope::delta::DeltaElement;
use xi_core::plugin_rpc::{TextUnit, GetDataResponse};

use plugin_base::{Error, DataSource};

const CHUNK_SIZE: usize = 1024 * 1024;

/// A simple cache, holding a single contiguous chunk of the document.
#[derive(Debug, Clone, Default)]
pub struct ChunkCache {
    /// The position of this chunk relative to the tracked document.
    /// All offsets are guaranteed to be valid UTF-8 character boundaries.
    pub offset: usize,
    /// A chunk of the remote buffer.
    pub contents: String,
    /// The (zero-based) line number of the line containing the start of the chunk.
    pub first_line: usize,
    /// The byte offset of the start of the chunk from the start of `first_line`.
    /// If this chunk starts at a line break, this will be 0.
    pub first_line_offset: usize,
    /// A list of indexes of newlines in this chunk.
    pub line_offsets: Vec<usize>,
    /// The total size of the tracked document.
    pub buf_size: usize,
    pub num_lines: usize,
    pub rev: u64,
}

impl ChunkCache {
    // TODO: remove?
    pub fn get_slice<DS: DataSource>(&mut self, source: &DS, start: usize, end: usize)
        -> Result<&str, Error>
    {
        loop {
            let chunk_start = self.offset;
            let chunk_end = chunk_start + self.contents.len();
            if start >= chunk_start && (start < chunk_end || chunk_end == self.buf_size) {
                // At least the first codepoint at start is in the chunk.
                if end < chunk_end || chunk_end == self.buf_size {
                    return Ok(&self.contents[start - chunk_start ..]);
                }
                let new_chunk = source.get_data(chunk_end, TextUnit::Utf8,
                                                CHUNK_SIZE, self.rev)?.chunk;
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
                self.contents = source.get_data(start, TextUnit::Utf8,
                                                CHUNK_SIZE, self.rev)?.chunk;
                self.offset = start;
            }
        }
    }

    pub fn get_line<DS>(&mut self, source: &DS, line_num: usize) -> Result<&str, Error>
        where DS: DataSource
    {
        if line_num > self.num_lines { return Err(Error::BadRequest) }

        // if chunk does not include the start of this line, fetch and reset everything
        if self.contents.len() == 0
            || line_num < self.first_line
            || (line_num == self.first_line && self.first_line_offset > 0) {
                let resp = source.get_data(line_num, TextUnit::Line, CHUNK_SIZE, self.rev)?;
                self.reset_chunk(resp);
        }

        // We now know that the start of this line is contained in self.contents.
        let mut start_off = self.cached_offset_of_line(line_num).unwrap();

        // Now we make sure we also contain the end of the line, fetching more
        // of the document as necessary.
        loop {
            if let Some(end_off) = self.cached_offset_of_line(line_num + 1) {
                return Ok(&self.contents[start_off..end_off])
            }
            // if we have a chunk and we're fetching more, discard unnecessary
            // portion of our chunk.
            if start_off != 0 {
                self.clear_up_to(start_off);
                start_off = 0;
            }

            let chunk_end = self.offset + self.contents.len();
            let resp = source.get_data(chunk_end, TextUnit::Utf8,
                                       CHUNK_SIZE, self.rev)?;
            self.append_chunk(resp);
        }
    }

    /// Returns the offset of the provided `line_num` in `self.contents` if
    /// it is present in the chunk.
    fn cached_offset_of_line(&self, line_num: usize) -> Option<usize> {
        if line_num < self.first_line { return None }

        let rel_line_num = line_num - self.first_line;

        if rel_line_num == 0 && self.first_line_offset == 0 {
            return Some(0)
        }
        if rel_line_num <= self.line_offsets.len() {
            return Some(self.line_offsets[rel_line_num - 1])
        }

        // EOF
        if line_num == self.num_lines && self.offset + self.contents.len() == self.buf_size {
            return Some(self.contents.len())
        }
        None
    }

    /// Clears anything in the cache up to `offset`, which is indexed relative
    /// to `self.contents`.
    ///
    /// # Panics
    ///
    /// Panics if `offset` is not a character boundary, or if `offset` is greater than
    /// the length of `self.content`.
    fn clear_up_to(&mut self, offset: usize) {
        if offset > self.contents.len() {
            panic!("offset greater than content length: {} > {}", offset, self.contents.len())
        }

        let new_contents = self.contents.split_off(offset);
        self.contents = new_contents;
        self.offset += offset;
        // first find out if offset is a line offset, and set first_line / first_line_offset
        let (new_line, new_line_off) = match self.line_offsets.binary_search(&offset) {
            Ok(idx) => (self.first_line + idx + 1, 0),
            Err(0) => (self.first_line, self.first_line_offset + offset),
            Err(idx) => (self.first_line + idx, offset - self.line_offsets[idx - 1]),
        };

        // then clear line_offsets up to and including offset
        self.line_offsets = self.line_offsets.iter()
            .filter(|i| **i > offset)
            .map(|i| i - offset)
            .collect();

        self.first_line = new_line;
        self.first_line_offset = new_line_off;
    }

    /// Discard any existing cache, starting again with the new data.
    fn reset_chunk(&mut self, data: GetDataResponse) {
        self.contents = data.chunk;
        self.offset = data.offset;
        self.first_line = data.first_line;
        self.first_line_offset = data.first_line_offset;
        self.recalculate_line_offsets();
    }

    /// Append to the existing cache, leaving existing data in place.
    fn append_chunk(&mut self, data: GetDataResponse) {
        self.contents.push_str(data.chunk.as_str());
        // this is doing extra work in the case where we're fetching a single
        // massive (multiple of CHUNK_SIZE) line, but unclear if it's worth optimizing
        self.recalculate_line_offsets();
    }

    fn recalculate_line_offsets(&mut self) {
        self.line_offsets.clear();
        // find position of newlines:
        let mut cur_idx = 0;
        while let Some(idx) = memchr(b'\n', &self.contents.as_bytes()[cur_idx..]) {
            self.line_offsets.push(cur_idx + idx + 1);
            cur_idx += idx + 1;
        }
    }

    /// Updates the chunk to reflect changes in this delta.
    pub fn apply_update(&mut self, new_len: usize, num_lines: usize,
                        rev: u64, delta: Option<&RopeDelta>) {
        self.buf_size = new_len;
        self.num_lines =  num_lines;
        self.rev = rev;
        let delta = match delta {
            Some(d) => d,
            None => return,
        };

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

    pub fn clear(&mut self) {
        self.contents.clear();
        self.offset = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xi_rope::interval::Interval;
    use xi_rope::delta::Delta;
    use xi_rope::rope::{Rope, LinesMetric};
    use xi_core::plugin_rpc::GetDataResponse;

    struct MockDataSource(Rope);

    impl DataSource for MockDataSource {
        fn get_data(&self, start: usize, unit: TextUnit, _max_size: usize, _rev: u64)
            -> Result<GetDataResponse, Error> {
            let offset = unit.resolve_offset(&self.0, start)
                .ok_or(Error::Other("unable to resolve offset".into()))?;
            let first_line = self.0.line_of_offset(offset);
            let first_line_offset = offset - self.0.offset_of_line(first_line);
            let end_off = (offset + CHUNK_SIZE).min(self.0.len());

            // not the right error, but okay for this
            if offset > self.0.len() {
                Err(Error::Other("offset too big".into()))
            } else {
                let chunk = self.0.slice_to_string(offset, end_off);
                Ok(GetDataResponse { chunk, offset, first_line, first_line_offset })
            }
        }
    }

    #[test]
    fn simple_chunk() {
        let mut c = ChunkCache::default();
        c.buf_size = 2;
        c.contents = "oh".into();

        let d = Delta::simple_edit(Interval::new_closed_open(0, 0), "yay".into(), c.contents.len());
        c.apply_update(d.new_document_len(), 1, 1, Some(&d));
        assert_eq!(&c.contents, "yayoh");
        assert_eq!(c.offset, 0);

        let d = Delta::simple_edit(Interval::new_closed_open(0, 0), "ahh".into(), c.contents.len());
        c.apply_update(d.new_document_len(), 1, 2, Some(&d));

        assert_eq!(&c.contents, "ahhyayoh");
        assert_eq!(c.offset, 0);

        let d = Delta::simple_edit(Interval::new_closed_open(2, 3), "oops".into(), c.contents.len());
        assert_eq!(d.els.len(), 3);
        c.apply_update(d.new_document_len(), 1, 3, Some(&d));

        assert_eq!(&c.contents, "ahoopsyayoh");
        assert_eq!(c.offset, 0);

        let d = Delta::simple_edit(Interval::new_closed_open(9, 9), "fin".into(), c.contents.len());
        c.apply_update(d.new_document_len(), 1, 5, Some(&d));

        assert_eq!(&c.contents, "ahoopsyayfinoh");
        assert_eq!(c.offset, 0);
    }


    #[test]
    fn offset_chunk() {
        let mut c = ChunkCache::default();
        c.offset = 10;
        c.contents = "tenchars!!".into();
        c.buf_size = 20;

        let d = Delta::simple_edit(Interval::new_closed_open(0, 0), "yay".into(),
                                   c.offset + c.contents.len());
        c.apply_update(d.new_document_len(), 1, 1, Some(&d));
        assert_eq!(c.offset, 13);
        assert_eq!(&c.contents, "tenchars!!");

        let d = Delta::simple_edit(Interval::new_closed_open(16, 0), "t".into(),
                                   c.offset + c.contents.len());
        c.apply_update(d.new_document_len(), 1, 2, Some(&d));
        assert_eq!(c.offset, 13);
        assert_eq!(&c.contents, "tentchars!!");

        let d = Delta::simple_edit(Interval::new_closed_open(5, 15), "stu".into(),
                                   c.offset + c.contents.len());
        c.apply_update(d.new_document_len(), 1, 3, Some(&d));
        assert_eq!(c.offset, 8);
        assert_eq!(&c.contents, "ntchars!!");

        // some edit off the end of the chunk
        let d = Delta::simple_edit(Interval::new_closed_open(50, 50), "hmm".into(),
                                   50);
        c.apply_update(d.new_document_len(), 1, 4, Some(&d));
        assert_eq!(c.offset, 8);
        assert_eq!(&c.contents, "ntchars!!");

        let d = Delta::simple_edit(Interval::new_closed_open(15, 17), "???".into(),
                                   c.offset + c.contents.len());
        c.apply_update(d.new_document_len(), 1, 5, Some(&d));
        assert_eq!(c.offset, 8);
        assert_eq!(&c.contents, "ntchars???");
    }

    #[test]
    fn get_lines() {
        let remote_document = MockDataSource("this\nhas\nfour\nlines!".into());
        let mut c = ChunkCache::default();
        c.buf_size = remote_document.0.len();
        c.num_lines = remote_document.0.measure::<LinesMetric>() + 1;
        assert_eq!(c.num_lines, 4);
        assert_eq!(c.buf_size, 20);
        assert_eq!(c.line_offsets.len(), 0);
        assert_eq!(c.get_line(&remote_document, 0).ok(), Some("this\n"));
        assert_eq!(c.line_offsets.len(), 3);
        assert_eq!(c.offset, 0);
        assert_eq!(c.buf_size, 20);
        assert_eq!(c.contents.len(), 20);
        assert_eq!(c.get_line(&remote_document, 2).ok(), Some("four\n"));
        assert_eq!(c.cached_offset_of_line(4), Some(20));
        assert_eq!(c.get_line(&remote_document, 3).ok(), Some("lines!"));
        assert!(c.get_line(&remote_document, 4).is_err());
    }

    #[test]
    fn reset_chunk() {
        let data = GetDataResponse {
            chunk: "1\n2\n3\n4\n5\n6\n7".into(),
            offset: 0,
            first_line: 0,
            first_line_offset: 0,
        };
        let mut cache = ChunkCache::default();
        cache.reset_chunk(data);

        assert_eq!(cache.line_offsets.len(), 6);
        assert_eq!(cache.line_offsets, vec![2, 4, 6, 8, 10, 12]);

        let idx_1 = cache.cached_offset_of_line(1).unwrap();
        let idx_2 = cache.cached_offset_of_line(2).unwrap();
        assert_eq!(&cache.contents.as_str()[idx_1..idx_2], "2\n");
    }

    #[test]
    fn clear_up_to() {
        let mut c = ChunkCache::default();
        let data = GetDataResponse {
            chunk: "this\n has a newline at idx 4\nand at idx 28".into(),
            offset: 0,
            first_line: 0,
            first_line_offset: 0,
        };
        c.reset_chunk(data);
        assert_eq!(c.line_offsets, vec![5, 29]);
        c.clear_up_to(5);
        assert_eq!(c.offset, 5);
        assert_eq!(c.first_line, 1);
        assert_eq!(c.first_line_offset, 0);
        assert_eq!(c.line_offsets, vec![24]);

        c.clear_up_to(10);
        assert_eq!(c.offset, 15);
        assert_eq!(c.first_line, 1);
        assert_eq!(c.first_line_offset, 10);
        assert_eq!(c.line_offsets, vec![14]);
    }
}
