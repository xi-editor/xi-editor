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

//! Data structures for tracking the state of the front-end's line cache
//! and preparing render plans to update it.

use std::cmp::{max, min};
use std::fmt;

const SCROLL_SLOP: usize = 2;
const PRESERVE_EXTENT: usize = 1000;

/// The line cache shadow tracks the state of the line cache in the front-end.
/// Any content marked as valid here is up-to-date in the current state of the
/// view. Also, if `dirty` is false, then the entire line cache is valid.
#[derive(Debug)]
pub struct LineCacheShadow {
    spans: Vec<Span>,
    dirty: bool,
}

type Validity = u8;

pub const INVALID: Validity = 0;
pub const TEXT_VALID: Validity = 1;
pub const STYLES_VALID: Validity = 2;
pub const CURSOR_VALID: Validity = 4;
pub const ALL_VALID: Validity = 7;

pub struct Span {
    /// Number of lines in this span. Units are visual lines in the
    /// current state of the view.
    pub n: usize,
    /// Starting line number. Units are visual lines in the front end's
    /// current cache state (i.e. the last one rendered). Note: this is
    /// irrelevant if validity is 0.
    pub start_line_num: usize,
    /// Validity of lines in this span, consisting of the above constants or'ed.
    pub validity: Validity,
}

/// Builder for `LineCacheShadow` object.
pub struct Builder {
    spans: Vec<Span>,
    dirty: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum RenderTactic {
    /// Discard all content for this span. Used to keep storage reasonable.
    Discard,
    /// Preserve existing content.
    Preserve,
    /// Render content if it is invalid.
    Render,
}

pub struct RenderPlan {
    /// Each span is a number of lines and a tactic.
    pub spans: Vec<(usize, RenderTactic)>,
}

pub struct PlanIterator<'a> {
    lc_shadow: &'a LineCacheShadow,
    plan: &'a RenderPlan,
    shadow_ix: usize,
    shadow_line_num: usize,
    plan_ix: usize,
    plan_line_num: usize,
}

pub struct PlanSegment {
    /// Line number of start of segment, visual lines in current view state.
    pub our_line_num: usize,
    /// Line number of start of segment, visual lines in client's cache, if validity != 0.
    pub their_line_num: usize,
    /// Number of visual lines in this segment.
    pub n: usize,
    /// Validity of this segment in client's cache.
    pub validity: Validity,
    /// Tactic for rendering this segment.
    pub tactic: RenderTactic,
}

impl Builder {
    pub fn new() -> Builder {
        Builder { spans: Vec::new(), dirty: false }
    }

    pub fn build(self) -> LineCacheShadow {
        LineCacheShadow { spans: self.spans, dirty: self.dirty }
    }

    pub fn add_span(&mut self, n: usize, start_line_num: usize, validity: Validity) {
        if n > 0 {
            if let Some(last) = self.spans.last_mut() {
                if last.validity == validity
                    && (validity == INVALID || last.start_line_num + last.n == start_line_num)
                {
                    last.n += n;
                    return;
                }
            }
            self.spans.push(Span { n, start_line_num, validity });
        }
    }

    pub fn set_dirty(&mut self, dirty: bool) {
        self.dirty = dirty;
    }
}

impl LineCacheShadow {
    pub fn edit(&mut self, start: usize, end: usize, replace: usize) {
        let mut b = Builder::new();
        let mut line_num = 0;
        let mut i = 0;
        while i < self.spans.len() {
            let span = &self.spans[i];
            if line_num + span.n <= start {
                b.add_span(span.n, span.start_line_num, span.validity);
                line_num += span.n;
                i += 1;
            } else {
                b.add_span(start - line_num, span.start_line_num, span.validity);
                break;
            }
        }
        b.add_span(replace, 0, INVALID);
        for span in &self.spans[i..] {
            if line_num + span.n > end {
                let offset = end.saturating_sub(line_num);
                b.add_span(span.n - offset, span.start_line_num + offset, span.validity);
            }
            line_num += span.n;
        }
        b.set_dirty(true);
        *self = b.build();
    }

    pub fn partial_invalidate(&mut self, start: usize, end: usize, invalid: Validity) {
        let mut clean = true;
        let mut line_num = 0;
        for span in &self.spans {
            if start < line_num + span.n && end > line_num && (span.validity & invalid) != 0 {
                clean = false;
                break;
            }
            line_num += span.n;
        }
        if clean {
            return;
        }

        let mut b = Builder::new();
        let mut line_num = 0;
        for span in &self.spans {
            if start > line_num {
                b.add_span(min(span.n, start - line_num), span.start_line_num, span.validity);
            }
            let invalid_start = max(start, line_num);
            let invalid_end = min(end, line_num + span.n);
            if invalid_end > invalid_start {
                b.add_span(
                    invalid_end - invalid_start,
                    span.start_line_num + (invalid_start - line_num),
                    span.validity & !invalid,
                );
            }
            if line_num + span.n > end {
                let offset = end.saturating_sub(line_num);
                b.add_span(span.n - offset, span.start_line_num + offset, span.validity);
            }
            line_num += span.n;
        }
        b.set_dirty(true);
        *self = b.build();
    }

    pub fn needs_render(&self, plan: &RenderPlan) -> bool {
        self.dirty
            || self
                .iter_with_plan(plan)
                .any(|seg| seg.tactic == RenderTactic::Render && seg.validity != ALL_VALID)
    }

    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    pub fn iter_with_plan<'a>(&'a self, plan: &'a RenderPlan) -> PlanIterator<'a> {
        PlanIterator {
            lc_shadow: self,
            plan,
            shadow_ix: 0,
            shadow_line_num: 0,
            plan_ix: 0,
            plan_line_num: 0,
        }
    }
}

impl Default for LineCacheShadow {
    fn default() -> LineCacheShadow {
        Builder::new().build()
    }
}

impl<'a> Iterator for PlanIterator<'a> {
    type Item = PlanSegment;

    fn next(&mut self) -> Option<PlanSegment> {
        if self.shadow_ix == self.lc_shadow.spans.len() || self.plan_ix == self.plan.spans.len() {
            return None;
        }
        let shadow_span = &self.lc_shadow.spans[self.shadow_ix];
        let plan_span = &self.plan.spans[self.plan_ix];
        let start = max(self.shadow_line_num, self.plan_line_num);
        let end = min(self.shadow_line_num + shadow_span.n, self.plan_line_num + plan_span.0);
        let result = PlanSegment {
            our_line_num: start,
            their_line_num: shadow_span.start_line_num + (start - self.shadow_line_num),
            n: end - start,
            validity: shadow_span.validity,
            tactic: plan_span.1,
        };

        if end == self.shadow_line_num + shadow_span.n {
            self.shadow_line_num = end;
            self.shadow_ix += 1;
        }
        if end == self.plan_line_num + plan_span.0 {
            self.plan_line_num = end;
            self.plan_ix += 1;
        }
        Some(result)
    }
}

impl RenderPlan {
    /// This function implements the policy of what to discard, what to preserve, and
    /// what to render.
    pub fn create(total_height: usize, first_line: usize, height: usize) -> RenderPlan {
        let mut spans = Vec::new();
        let mut last = 0;
        let first_line = min(first_line, total_height);
        if first_line > PRESERVE_EXTENT {
            last = first_line - PRESERVE_EXTENT;
            spans.push((last, RenderTactic::Discard));
        }
        if first_line > SCROLL_SLOP {
            let n = first_line - SCROLL_SLOP - last;
            spans.push((n, RenderTactic::Preserve));
            last += n;
        }
        let render_end = min(first_line + height + SCROLL_SLOP, total_height);
        spans.push((render_end - last, RenderTactic::Render));
        last = render_end;
        let preserve_end = min(first_line + height + PRESERVE_EXTENT, total_height);
        if preserve_end > last {
            spans.push((preserve_end - last, RenderTactic::Preserve));
            last = preserve_end;
        }
        if total_height > last {
            spans.push((total_height - last, RenderTactic::Discard));
        }
        RenderPlan { spans }
    }

    /// Upgrade a range of lines to the "Render" tactic.
    pub fn request_lines(&mut self, start: usize, end: usize) {
        let mut spans: Vec<(usize, RenderTactic)> = Vec::new();
        let mut i = 0;
        let mut line_num = 0;
        while i < self.spans.len() {
            let span = &self.spans[i];
            if line_num + span.0 <= start {
                spans.push(*span);
                line_num += span.0;
                i += 1;
            } else {
                if line_num < start {
                    spans.push((start - line_num, span.1));
                }
                break;
            }
        }
        spans.push((end - start, RenderTactic::Render));
        for span in &self.spans[i..] {
            if line_num + span.0 > end {
                let offset = end.saturating_sub(line_num);
                spans.push((span.0 - offset, span.1));
            }
            line_num += span.0;
        }
        self.spans = spans;
    }
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let validity = match self.validity {
            TEXT_VALID => "text",
            ALL_VALID => "all",
            _other => "mixed",
        };
        if self.validity == INVALID {
            write!(f, "({} invalid)", self.n)?;
        } else {
            write!(f, "({}: {}, {})", self.start_line_num, self.n, validity)?;
        }
        Ok(())
    }
}
