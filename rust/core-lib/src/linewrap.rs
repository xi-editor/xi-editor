// Copyright 2016 The xi-editor Authors.
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

//! Compute line wrapping breaks for text.

use std::ops::Range;

use xi_rope::breaks::{BreakBuilder, Breaks, BreaksBaseMetric, BreaksInfo, BreaksMetric};
use xi_rope::spans::Spans;
use xi_rope::{Cursor, LinesMetric, Rope, RopeDelta, RopeInfo};
use xi_trace::trace_block;
use xi_unicode::LineBreakLeafIter;

use crate::client::Client;
use crate::styles::{Style, N_RESERVED_STYLES};
use crate::width_cache::{CodepointMono, Token, WidthCache, WidthMeasure};

/// The visual width of the buffer for the purpose of word wrapping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum WrapWidth {
    /// No wrapping in effect.
    None,

    /// Width in bytes (utf-8 code units).
    ///
    /// Only works well for ASCII, will probably not be maintained long-term.
    Bytes(usize),

    /// Width in px units, requiring measurement by the front-end.
    Width(f64),
}

impl Default for WrapWidth {
    fn default() -> Self {
        WrapWidth::None
    }
}

/// Tracks state related to visual lines.
#[derive(Default)]
pub(crate) struct Lines {
    breaks: Breaks,
    wrap: WrapWidth,
}

/// A range of bytes representing a visual line.
pub(crate) type VisualLine = Range<usize>;

impl Lines {
    pub(crate) fn set_wrap_width(&mut self, wrap: WrapWidth) {
        if wrap != self.wrap {
            self.wrap = wrap;
            //TODO: for incremental, we clear breaks and update frontier.
        }
    }

    pub(crate) fn has_soft_breaks(&self) -> bool {
        self.wrap != WrapWidth::None
    }

    pub(crate) fn visual_line_of_offset(&self, text: &Rope, offset: usize) -> usize {
        match self.wrap {
            WrapWidth::None => text.line_of_offset(offset),
            _ => self.breaks.convert_metrics::<BreaksBaseMetric, BreaksMetric>(offset),
        }
    }

    /// Returns the byte offset corresponding to the line `line`.
    pub(crate) fn offset_of_visual_line(&self, text: &Rope, line: usize) -> usize {
        match self.wrap {
            WrapWidth::None => {
                // sanitize input
                let line = line.min(text.measure::<LinesMetric>() + 1);
                text.offset_of_line(line)
            }
            _ => self.breaks.convert_metrics::<BreaksMetric, BreaksBaseMetric>(line),
        }
    }

    /// Returns an iterator over [`VisualLine`]s, starting at (and including)
    /// `start_line`.
    pub(crate) fn iter_lines<'a>(
        &'a self,
        text: &'a Rope,
        start_line: usize,
    ) -> impl Iterator<Item = VisualLine> + 'a {
        let offset = self.offset_of_visual_line(text, start_line);
        let cursor = MergedBreaks::new(text, &self.breaks, offset);
        VisualLines { offset, cursor, len: text.len(), eof: false }
    }

    // TODO: this is where the incremental goes
    /// Calculates new breaks for a chunk of the document.
    pub(crate) fn rewrap_chunk(
        &mut self,
        text: &Rope,
        width_cache: &mut WidthCache,
        client: &Client,
        spans: &Spans<Style>,
    ) {
        self.breaks = match self.wrap {
            WrapWidth::None => Breaks::new_no_break(text.len()),
            WrapWidth::Bytes(c) => rewrap_all(text, width_cache, spans, &CodepointMono, c as f64),
            WrapWidth::Width(w) => rewrap_all(text, width_cache, spans, client, w),
        };
    }

    /// Updates breaks as necessary after an edit.
    pub(crate) fn after_edit(
        &mut self,
        text: &Rope,
        delta: &RopeDelta,
        width_cache: &mut WidthCache,
        client: &Client,
    ) {
        let _t = trace_block("Lines::after_edit", &["core"]);

        let (iv, newsize) = delta.summary();
        let mut builder = BreakBuilder::new();
        builder.add_no_break(newsize);
        self.breaks.edit(iv, builder.build());

        let mut start = iv.start;
        let end = start + newsize;
        let mut cursor = Cursor::new(&text, start);
        start = cursor.at_or_prev::<LinesMetric>().unwrap_or(0);

        let new_breaks = match self.wrap {
            WrapWidth::None => Breaks::new_no_break(newsize),
            WrapWidth::Bytes(c) => compute_rewrap(
                text,
                width_cache,
                &CodepointMono,
                c as f64,
                &self.breaks,
                start,
                end,
            ),
            WrapWidth::Width(w) => {
                compute_rewrap(text, width_cache, client, w, &self.breaks, start, end)
            }
        };

        let edit_end = start + new_breaks.len();
        self.breaks.edit(start..edit_end, new_breaks);
    }
}

/// A potential opportunity to insert a break. In this representation, the widths
/// have been requested (in a batch request) but are not necessarily known until
/// the request is issued.
struct PotentialBreak {
    /// The offset within the text of the end of the word.
    pos: usize,
    /// A token referencing the width of the word, to be resolved in the width cache.
    tok: Token,
    /// Whether the break is a hard break or a soft break.
    hard: bool,
}

/// State for a rewrap in progress
struct RewrapCtx<'a, T: 'a> {
    text: &'a Rope,
    lb_cursor: LineBreakCursor<'a>,
    lb_cursor_pos: usize,
    width_cache: &'a mut WidthCache,
    client: &'a T,
    pot_breaks: Vec<PotentialBreak>,
    /// Index within `pot_breaks`
    pot_break_ix: usize,
    /// Offset of maximum break (ie hard break following edit)
    max_offset: usize,
    max_width: f64,
}

// This constant should be tuned so that the RPC takes about 1ms. Less than that,
// RPC overhead becomes significant. More than that, interactivity suffers.
const MAX_POT_BREAKS: usize = 10_000;

impl<'a, T: WidthMeasure> RewrapCtx<'a, T> {
    fn new(
        text: &'a Rope,
        /* _style_spans: &Spans<Style>, */ client: &'a T,
        max_width: f64,
        width_cache: &'a mut WidthCache,
        start: usize,
        end: usize,
    ) -> RewrapCtx<'a, T> {
        let lb_cursor_pos = start;
        let lb_cursor = LineBreakCursor::new(text, start);
        RewrapCtx {
            text,
            lb_cursor,
            lb_cursor_pos,
            width_cache,
            client,
            pot_breaks: Vec::new(),
            pot_break_ix: 0,
            max_offset: end,
            max_width,
        }
    }

    fn refill_pot_breaks(&mut self) {
        let mut req = self.width_cache.batch_req();

        self.pot_breaks.clear();
        self.pot_break_ix = 0;
        let mut pos = self.lb_cursor_pos;
        while pos < self.max_offset && self.pot_breaks.len() < MAX_POT_BREAKS {
            let (next, hard) = self.lb_cursor.next();
            // TODO: avoid allocating string
            let word = self.text.slice_to_cow(pos..next);
            let tok = req.request(N_RESERVED_STYLES, &word);
            pos = next;
            self.pot_breaks.push(PotentialBreak { pos, tok, hard });
        }
        req.resolve_pending(self.client).unwrap();
        self.lb_cursor_pos = pos;
    }

    /// Compute the next break, assuming `start` is a valid break.
    ///
    /// Invariant: `start` corresponds to the start of the word referenced by `pot_break_ix`.
    fn wrap_one_line(&mut self, start: usize) -> Option<usize> {
        let mut line_width = 0.0;
        let mut pos = start;
        while pos < self.max_offset {
            if self.pot_break_ix >= self.pot_breaks.len() {
                self.refill_pot_breaks();
            }
            let pot_break = &self.pot_breaks[self.pot_break_ix];
            let width = self.width_cache.resolve(pot_break.tok);
            if !pot_break.hard {
                if line_width == 0.0 && width >= self.max_width {
                    self.pot_break_ix += 1;
                    return Some(pot_break.pos);
                }
                line_width += width;
                if line_width > self.max_width {
                    return Some(pos);
                }
                self.pot_break_ix += 1;
                pos = pot_break.pos;
            } else if line_width != 0. && width + line_width > self.max_width {
                // if this is a hard break but we would have broken at the previous
                // pos otherwise, we still break at the previous pos.
                return Some(pos);
            } else {
                self.pot_break_ix += 1;
                return Some(pot_break.pos);
            }
        }
        None
    }
}

/// Wrap the text (in batch mode) using width measurement.
fn rewrap_all<T: WidthMeasure>(
    text: &Rope,
    width_cache: &mut WidthCache,
    _style_spans: &Spans<Style>,
    client: &T,
    max_width: f64,
) -> Breaks {
    let mut ctx =
        RewrapCtx::new(text, /* style_spans, */ client, max_width, width_cache, 0, text.len());
    let mut builder = BreakBuilder::new();
    let mut pos = 0;
    while let Some(next) = ctx.wrap_one_line(pos) {
        builder.add_break(next - pos);
        pos = next;
    }
    builder.add_no_break(text.len() - pos);
    builder.build()
}

//NOTE: incremental version of rewrap_all
/// Compute a new chunk of breaks after an edit. Returns new breaks to replace
/// the old ones. The interval [start..end] represents a frontier.
fn compute_rewrap<T: WidthMeasure>(
    text: &Rope,
    width_cache: &mut WidthCache,
    /* style_spans: &Spans<Style>, */ client: &T,
    max_width: f64,
    breaks: &Breaks,
    start: usize,
    end: usize,
) -> Breaks {
    let mut line_cursor = Cursor::new(&text, end);
    let measure_end = line_cursor.next::<LinesMetric>().unwrap_or(text.len());
    let mut ctx = RewrapCtx::new(
        text,
        /* style_spans, */ client,
        max_width,
        width_cache,
        start,
        measure_end,
    );
    let mut builder = BreakBuilder::new();
    let mut pos = start;
    let mut break_cursor = Cursor::new(&breaks, end);
    let mut next_break = break_cursor.at_or_next::<BreaksBaseMetric>();
    loop {
        // iterate newly computed breaks and existing breaks until they converge
        if let Some(new_next) = ctx.wrap_one_line(pos) {
            while let Some(old_next) = next_break {
                if old_next >= new_next {
                    break;
                }
                next_break = break_cursor.next::<BreaksBaseMetric>();
            }
            // TODO: we might be able to tighten the logic, avoiding this last break,
            // in some cases (resulting in a smaller delta).
            builder.add_break(new_next - pos);
            if let Some(old_next) = next_break {
                if new_next == old_next {
                    // Breaking process has converged.
                    break;
                }
            }
            pos = new_next;
        } else {
            // EOF
            builder.add_no_break(text.len() - pos);
            break;
        }
    }
    builder.build()
}

struct LineBreakCursor<'a> {
    inner: Cursor<'a, RopeInfo>,
    lb_iter: LineBreakLeafIter,
    last_byte: u8,
}

impl<'a> LineBreakCursor<'a> {
    fn new(text: &'a Rope, pos: usize) -> LineBreakCursor<'a> {
        let inner = Cursor::new(text, pos);
        let lb_iter = match inner.get_leaf() {
            Some((s, offset)) => LineBreakLeafIter::new(s.as_str(), offset),
            _ => LineBreakLeafIter::default(),
        };
        LineBreakCursor { inner, lb_iter, last_byte: 0 }
    }

    // position and whether break is hard; up to caller to stop calling after EOT
    fn next(&mut self) -> (usize, bool) {
        let mut leaf = self.inner.get_leaf();
        loop {
            match leaf {
                Some((s, offset)) => {
                    let (next, hard) = self.lb_iter.next(s.as_str());
                    if next < s.len() {
                        return (self.inner.pos() - offset + next, hard);
                    }
                    if !s.is_empty() {
                        self.last_byte = s.as_bytes()[s.len() - 1];
                    }
                    leaf = self.inner.next_leaf();
                }
                // A little hacky but only reports last break as hard if final newline
                None => return (self.inner.pos(), self.last_byte == b'\n'),
            }
        }
    }
}

struct VisualLines<'a> {
    cursor: MergedBreaks<'a>,
    offset: usize,
    len: usize,
    eof: bool,
}

impl<'a> Iterator for VisualLines<'a> {
    type Item = VisualLine;

    fn next(&mut self) -> Option<VisualLine> {
        let next_end_bound = match self.cursor.next() {
            Some(b) => b,
            None if self.eof => return None,
            _else => {
                self.eof = true;
                self.len
            }
        };
        let result = self.offset..next_end_bound;
        self.offset = next_end_bound;
        Some(result)
    }
}

/// A cursor over both hard and soft breaks. Currently this is either/or,
/// but eventually soft will be soft only, and this will interleave them.
struct MergedBreaks<'a> {
    text: Cursor<'a, RopeInfo>,
    soft: Cursor<'a, BreaksInfo>,
    offset: usize,
}

impl<'a> Iterator for MergedBreaks<'a> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        // TODO: remove when soft and hard are separate
        let prev_off = self.offset;
        self.offset = if self.soft.root().measure::<BreaksMetric>() != 0 {
            self.soft.next::<BreaksMetric>()
        } else {
            self.text.next::<LinesMetric>()
        }?;

        // if we're at EOF, we only send a break if there's an actual trailing newline.
        let eof_without_newline = self.offset > 0
            && self.offset == self.text.total_len()
            && self.text.get_leaf().map(|(l, _)| l.as_bytes()[l.len() - 1] != b'\n').unwrap();
        if self.offset == prev_off || eof_without_newline {
            None
        } else {
            Some(self.offset)
        }
    }
}

impl<'a> MergedBreaks<'a> {
    fn new(text: &'a Rope, breaks: &'a Breaks, offset: usize) -> Self {
        debug_assert_eq!(text.len(), breaks.len());
        let text = Cursor::new(text, offset);
        let soft = Cursor::new(breaks, offset);
        MergedBreaks { text, soft, offset }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::width_cache::CodepointMono;
    use std::borrow::Cow;

    fn debug_breaks<'a>(text: &'a Rope, width: f64) -> Vec<Cow<'a, str>> {
        let mut result = Vec::new();
        let mut width_cache = WidthCache::new();
        let spans: Spans<Style> = Spans::default();
        let breaks = match width {
            w if w == 0. => Breaks::new_no_break(text.len()),
            w => rewrap_all(text, &mut width_cache, &spans, &CodepointMono, w),
        };
        let lines = Lines { breaks, wrap: WrapWidth::Bytes(width as usize) };
        for line in lines.iter_lines(text, 0) {
            result.push(text.slice_to_cow(line));
        }
        result
    }

    #[test]
    fn column_breaks_basic() {
        let text: Rope = "every wordthing should getits own".into();
        let result = debug_breaks(&text, 8.0);
        assert_eq!(result, vec!["every ", "wordthing ", "should ", "getits ", "own",]);
    }

    #[test]
    fn column_breaks_trailing_newline() {
        let text: Rope = "every wordthing should getits ow\n".into();
        let result = debug_breaks(&text, 8.0);
        assert_eq!(result, vec!["every ", "wordthing ", "should ", "getits ", "ow\n", "",]);
    }

    #[test]
    fn soft_before_hard() {
        let text: Rope = "create abreak between THESE TWO\nwords andbreakcorrectlyhere\nplz".into();
        let result = debug_breaks(&text, 4.0);
        assert_eq!(
            result,
            vec![
                "create ",
                "abreak ",
                "between ",
                "THESE ",
                "TWO\n",
                "words ",
                "andbreakcorrectlyhere\n",
                "plz",
            ]
        );
    }

    #[test]
    fn column_breaks_hard_soft() {
        let text: Rope = "so\nevery wordthing should getits own".into();
        let result = debug_breaks(&text, 4.0);
        assert_eq!(result, vec!["so\n", "every ", "wordthing ", "should ", "getits ", "own",]);
    }

    #[test]
    fn empty_file() {
        let text: Rope = "".into();
        let result = debug_breaks(&text, 4.0);
        assert_eq!(result, vec![""]);
    }

    #[test]
    fn dont_break_til_i_tell_you() {
        let text: Rope = "thisis_longerthan_our_break_width".into();
        let result = debug_breaks(&text, 12.0);
        assert_eq!(result, vec!["thisis_longerthan_our_break_width"]);
    }

    #[test]
    fn break_now_though() {
        let text: Rope = "thisis_longerthan_our_break_width hi".into();
        let result = debug_breaks(&text, 12.0);
        assert_eq!(result, vec!["thisis_longerthan_our_break_width ", "hi"]);
    }

    #[test]
    fn newlines() {
        let text: Rope = "\n\n".into();
        let result = debug_breaks(&text, 4.0);
        assert_eq!(result, vec!["\n", "\n", ""]);
    }

    #[test]
    fn newline_eof() {
        let text: Rope = "hello\n".into();
        let result = debug_breaks(&text, 4.0);
        assert_eq!(result, vec!["hello\n", ""]);
    }

    #[test]
    fn no_newline_eof() {
        let text: Rope = "hello".into();
        let result = debug_breaks(&text, 4.0);
        assert_eq!(result, vec!["hello"]);
    }
}
