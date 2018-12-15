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

use std::cmp::Ordering;
use std::ops::Range;

use xi_rope::breaks::{BreakBuilder, Breaks, BreaksBaseMetric, BreaksInfo, BreaksMetric};
use xi_rope::rope::BaseMetric;
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
        let mut line = text.line_of_offset(offset);
        if self.wrap != WrapWidth::None {
            line += self.breaks.convert_metrics::<BreaksBaseMetric, BreaksMetric>(offset)
        }
        line
    }

    /// Returns the byte offset corresponding to the line `line`.
    pub(crate) fn offset_of_visual_line(&self, text: &Rope, line: usize) -> usize {
        match self.wrap {
            WrapWidth::None => {
                // sanitize input
                let line = line.min(text.measure::<LinesMetric>() + 1);
                text.offset_of_line(line)
            }
            _ => {
                let mut cursor = MergedBreaks::new(text, &self.breaks);
                cursor.offset_of_line(line)
            }
        }
    }

    /// Returns an iterator over [`VisualLine`]s, starting at (and including)
    /// `start_line`.
    pub(crate) fn iter_lines<'a>(
        &'a self,
        text: &'a Rope,
        start_line: usize,
    ) -> impl Iterator<Item = VisualLine> + 'a {
        let mut cursor = MergedBreaks::new(text, &self.breaks);
        let offset = cursor.offset_of_line(start_line);
        cursor.set_offset(offset);
        VisualLines { offset, cursor, len: text.len(), eof: false }
    }

    // TODO: this is where the incremental goes
    /// Calculates new breaks for a chunk of the document.
    pub(crate) fn rewrap_chunk(
        &mut self,
        text: &Rope,
        width_cache: &mut WidthCache,
        client: &Client,
        _spans: &Spans<Style>,
    ) {
        self.breaks = match self.wrap {
            WrapWidth::None => Breaks::new_no_break(text.len()),
            WrapWidth::Bytes(c) => rewrap_all(text, width_cache, &CodepointMono, c as f64),
            WrapWidth::Width(w) => rewrap_all(text, width_cache, client, w),
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
fn rewrap_all<T>(text: &Rope, width_cache: &mut WidthCache, client: &T, width: f64) -> Breaks
where
    T: WidthMeasure,
{
    let empty_breaks = Breaks::new_no_break(text.len());
    compute_rewrap(text, width_cache, client, width, &empty_breaks, 0, text.len())
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
            line_cursor.set(new_next);
            let is_hard = line_cursor.is_boundary::<LinesMetric>();
            while let Some(old_next) = next_break {
                if old_next >= new_next {
                    break;
                }
                next_break = break_cursor.next::<BreaksBaseMetric>();
            }
            // TODO: we might be able to tighten the logic, avoiding this last break,
            // in some cases (resulting in a smaller delta).
            if is_hard {
                builder.add_no_break(new_next - pos);
            } else {
                builder.add_break(new_next - pos);
            }
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

/// A cursor over both hard and soft breaks. Hard breaks are retrieved from
/// the rope; the soft breaks are stored independently; this interleaves them.
///
/// # Invariants:
///
/// `self.offset` is always a valid break in one of the cursors, unless
/// at 0 or EOF.
///
/// `self.offset == self.text.pos().min(self.soft.pos())`.
struct MergedBreaks<'a> {
    text: Cursor<'a, RopeInfo>,
    soft: Cursor<'a, BreaksInfo>,
    offset: usize,
    /// Starting from zero, how many calls to `next` to get to `self.offset`?
    cur_line: usize,
    total_lines: usize,
    /// Total length, in base units
    len: usize,
}

impl<'a> Iterator for MergedBreaks<'a> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        if self.text.pos() == self.offset && !self.at_eof() {
            // don't iterate past EOF, or we can't get the leaf and check for \n
            self.text.next::<LinesMetric>();
        }
        if self.soft.pos() == self.offset {
            self.soft.next::<BreaksMetric>();
        }
        let prev_off = self.offset;
        self.offset = self.text.pos().min(self.soft.pos());

        let eof_without_newline = self.offset > 0 && self.at_eof() && self.eof_without_newline();
        if self.offset == prev_off || eof_without_newline {
            None
        } else {
            self.cur_line += 1;
            Some(self.offset)
        }
    }
}

// arrived at this by just trying out a bunch of values ¯\_(ツ)_/¯
/// how far away a line can be before we switch to a binary search
const MAX_LINEAR_DIST: usize = 20;

impl<'a> MergedBreaks<'a> {
    fn new(text: &'a Rope, breaks: &'a Breaks) -> Self {
        debug_assert_eq!(text.len(), breaks.len());
        let text = Cursor::new(text, 0);
        let soft = Cursor::new(breaks, 0);
        let total_lines =
            text.root().measure::<LinesMetric>() + soft.root().measure::<BreaksMetric>();
        let len = text.total_len();
        MergedBreaks { text, soft, offset: 0, cur_line: 0, total_lines, len }
    }

    /// Sets the `self.offset` to the first valid break immediately at or preceding `offset`,
    /// and restores invariants.
    fn set_offset(&mut self, offset: usize) {
        self.text.set(offset);
        self.soft.set(offset);
        self.text.at_or_prev::<LinesMetric>();
        self.soft.at_or_prev::<BreaksMetric>();

        // self.offset should be at the first valid break immediately preceding `offset`, or 0.
        // the position of the non-break cursor should be > than that of the break cursor, or EOF.
        match self.text.pos().cmp(&self.soft.pos()) {
            Ordering::Less => {
                self.text.next::<LinesMetric>();
            }
            Ordering::Greater => {
                self.soft.next::<BreaksMetric>();
            }
            Ordering::Equal => assert!(self.text.pos() == 0),
        }

        self.offset = self.text.pos().min(self.soft.pos());
        self.cur_line = merged_line_of_offset(self.text.root(), self.soft.root(), self.offset);
    }

    fn offset_of_line(&mut self, line: usize) -> usize {
        match line {
            0 => 0,
            l if l >= self.total_lines => self.text.total_len(),
            l if l == self.cur_line => self.offset,
            l if l > self.cur_line && l - self.cur_line < MAX_LINEAR_DIST => {
                self.offset_of_line_linear(l)
            }
            other => self.offset_of_line_bsearch(other),
        }
    }

    fn offset_of_line_linear(&mut self, line: usize) -> usize {
        assert!(line > self.cur_line);
        let dist = line - self.cur_line;
        self.nth(dist - 1).unwrap_or(self.len)
    }

    fn offset_of_line_bsearch(&mut self, line: usize) -> usize {
        let mut range = 0..self.len;
        loop {
            let pivot = range.start + (range.end - range.start) / 2;
            self.set_offset(pivot);

            match self.cur_line {
                l if l == line => break self.offset,
                l if l > line => range = range.start..pivot,
                l if line - l > MAX_LINEAR_DIST => range = pivot..range.end,
                _else => break self.offset_of_line_linear(line),
            }
        }
    }

    fn at_eof(&self) -> bool {
        self.offset == self.len
    }

    fn eof_without_newline(&self) -> bool {
        debug_assert!(self.at_eof());
        self.text.get_leaf().map(|(l, _)| l.as_bytes().last() != Some(&b'\n')).unwrap()
    }
}

fn merged_line_of_offset(text: &Rope, soft: &Breaks, offset: usize) -> usize {
    text.convert_metrics::<BaseMetric, LinesMetric>(offset)
        + soft.convert_metrics::<BreaksBaseMetric, BreaksMetric>(offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::width_cache::CodepointMono;
    use std::borrow::Cow;
    use std::iter;

    fn make_lines(text: &Rope, width: f64) -> Lines {
        let mut width_cache = WidthCache::new();
        let wrap = WrapWidth::Bytes(width as usize);
        let breaks = rewrap_all(text, &mut width_cache, &CodepointMono, width);
        Lines { breaks, wrap }
    }

    fn debug_breaks<'a>(text: &'a Rope, width: f64) -> Vec<Cow<'a, str>> {
        let lines = make_lines(text, width);
        let result = lines.iter_lines(text, 0).map(|l| text.slice_to_cow(l)).collect();
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

    #[test]
    fn merged_offset() {
        let text: Rope = "a quite\nshort text".into();
        let mut builder = BreakBuilder::new();
        builder.add_break(2);
        builder.add_no_break(text.len() - 2);
        let breaks = builder.build();
        assert_eq!(merged_line_of_offset(&text, &breaks, 0), 0);
        assert_eq!(merged_line_of_offset(&text, &breaks, 1), 0);
        assert_eq!(merged_line_of_offset(&text, &breaks, 2), 1);
        assert_eq!(merged_line_of_offset(&text, &breaks, 5), 1);
        assert_eq!(merged_line_of_offset(&text, &breaks, 5), 1);
        assert_eq!(merged_line_of_offset(&text, &breaks, 9), 2);
        assert_eq!(merged_line_of_offset(&text, &breaks, text.len()), 2);

        let text: Rope = "a quite\nshort tex\n".into();
        // trailing newline increases total count
        assert_eq!(merged_line_of_offset(&text, &breaks, text.len()), 3);
    }

    #[test]
    fn bsearch_equivalence() {
        let text: Rope =
            iter::repeat("this is a line with some text in it, which is not unusual\n")
                .take(1000)
                .collect::<String>()
                .into();
        let mut width_cache = WidthCache::new();
        let breaks = rewrap_all(&text, &mut width_cache, &CodepointMono, 30.);

        let mut linear = MergedBreaks::new(&text, &breaks);
        let mut binary = MergedBreaks::new(&text, &breaks);

        // skip zero because these two impls don't handle edge cases
        for i in 1..1000 {
            linear.set_offset(0);
            binary.set_offset(0);
            assert_eq!(
                linear.offset_of_line_linear(i),
                binary.offset_of_line_bsearch(i),
                "line {}",
                i
            );
        }
    }
    #[test]
    fn set_offset() {
        let text: Rope = "aaaa\nbb bb cc\ncc dddd eeee ff\nff gggg".into();
        let lines = make_lines(&text, 2.);
        let mut merged = MergedBreaks::new(&text, &lines.breaks);

        let check_props = |m: &MergedBreaks, line, off, softpos, hardpos| {
            assert_eq!(m.cur_line, line);
            assert_eq!(m.offset, off);
            assert_eq!(m.soft.pos(), softpos);
            assert_eq!(m.text.pos(), hardpos);
        };
        merged.next();
        check_props(&merged, 1, 5, 8, 5);
        merged.set_offset(0);
        check_props(&merged, 0, 0, 0, 0);
        merged.set_offset(5);
        check_props(&merged, 1, 5, 8, 5);
        merged.set_offset(0);
        merged.set_offset(6);
        check_props(&merged, 1, 5, 8, 5);
        merged.set_offset(9);
        check_props(&merged, 2, 8, 8, 14);
        merged.set_offset(text.len());
        check_props(&merged, 10, 37, 37, 37);
        merged.set_offset(text.len() - 1);
        check_props(&merged, 9, 33, 33, 37);
    }

    #[test]
    fn test_break_at_linear_transition() {
        // do we handle the break at MAX_LINEAR_DIST correctly?
        let text = "a b c d e f g h i j k l m n o p q r s t u v w x ".into();
        let lines = make_lines(&text, 1.);

        for offset in 0..text.len() {
            let line = lines.visual_line_of_offset(&text, offset);
            let line_offset = lines.offset_of_visual_line(&text, line);
            assert!(line_offset <= offset, "{} <= {} L{} O{}", line_offset, offset, line, offset);
        }
    }

    #[test]
    fn iter_lines() {
        let text: Rope = "aaaa\nbb bb cc\ncc dddd eeee ff\nff gggg".into();
        let lines = make_lines(&text, 2.);
        let r: Vec<_> = lines.iter_lines(&text, 0).take(2).map(|l| text.slice_to_cow(l)).collect();
        assert_eq!(r, vec!["aaaa\n", "bb "]);

        let r: Vec<_> = lines.iter_lines(&text, 1).take(2).map(|l| text.slice_to_cow(l)).collect();
        assert_eq!(r, vec!["bb ", "bb "]);

        let r: Vec<_> = lines.iter_lines(&text, 3).take(3).map(|l| text.slice_to_cow(l)).collect();
        assert_eq!(r, vec!["cc\n", "cc ", "dddd "]);
    }
}
