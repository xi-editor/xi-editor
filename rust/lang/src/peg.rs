// Copyright 2017 Google Inc. All rights reserved.
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

//! Simple parser expression generator

use std::char::from_u32;
use std::ops;

pub trait Peg {
    fn p(&self, s: &[u8]) -> Option<usize>;
}

impl<F: Fn(&[u8]) -> Option<usize>> Peg for F {
    fn p(&self, s: &[u8]) -> Option<usize> {
        self(s)
    }
}

pub struct OneByte<F>(pub F);

impl<F: Fn(u8) -> bool> Peg for OneByte<F> {
    fn p(&self, s: &[u8]) -> Option<usize> {
        if s.is_empty() || !self.0(s[0]) {
            None
        } else {
            Some(1)
        }
    }
}

impl Peg for u8 {
    fn p(&self, s: &[u8]) -> Option<usize> {
        OneByte(|b| b == *self).p(s)
    }
}

pub struct OneChar<F>(pub F);

fn decode_utf8(s: &[u8]) -> Option<(char, usize)> {
    if s.is_empty() {
        return None;
    }
    let b = s[0];
    if b < 0x80 {
        return Some((b as char, 1));
    } else if b >= 0xc2 && b < 0xe0 && s.len() >= 2 {
        let b2 = s[1];
        if (b2 as i8) > -0x40 { return None; }
        let cp = ((b as u32) << 6) + (b2 as u32) - 0x3080;
        return from_u32(cp).map(|ch| (ch, 2));
    } else if b >= 0xe0 && b < 0xf0 && s.len() >= 3 {
        let b2 = s[1];
        let b3 = s[2];
        if (b2 as i8) > -0x40 || (b3 as i8) > -0x40 { return None; }
        let cp = ((b as u32) << 12) + ((b2 as u32) << 6) + (b3 as u32) - 0xe2080;
        if cp < 0x800 { return None; } // overlong encoding
        return from_u32(cp).map(|ch| (ch, 3));
    } else if b >= 0xf0 && b < 0xf5 && s.len() >= 4 {
        let b2 = s[1];
        let b3 = s[2];
        let b4 = s[3];
        if (b2 as i8) > -0x40 || (b3 as i8) > -0x40 || (b4 as i8) > -0x40 {
            return None;
        }
        let cp = ((b as u32) << 18) + ((b2 as u32) << 12) + ((b3 as u32) << 6)
            + (b4 as u32) - 0x3c82080;
        if cp < 0x10000 { return None; } // overlong encoding
        return from_u32(cp).map(|ch| (ch, 4));
    }
    None
}

impl<F: Fn(char) -> bool> Peg for OneChar<F> {
    fn p(&self, s: &[u8]) -> Option<usize> {
        if let Some((ch, len)) = decode_utf8(s) {
            if self.0(ch) {
                return Some(len)
            }
        }
        None
    }
}

// split out into a separate function to help inlining heuristics; even so,
// prefer to use bytes even though they're not quite as ergonomic
fn char_helper(s: &[u8], c: char) -> Option<usize> {
    OneChar(|x| x == c).p(s)
}

impl Peg for char {
    fn p(&self, s: &[u8]) -> Option<usize> {
        let c = *self;
        if c <= '\x7f' {
            (c as u8).p(s)
        } else {
            char_helper(s, c)
        }
    }
}

impl<'a> Peg for &'a [u8] {
    fn p(&self, s: &[u8]) -> Option<usize> {
        let len = self.len();
        if s.len() >= len && &s[..len] == *self {
            Some(len)
        } else {
            None
        }
    }
}

impl<'a> Peg for &'a str {
    fn p(&self, s: &[u8]) -> Option<usize> {
        self.as_bytes().p(s)
    }
}

impl<P1: Peg, P2: Peg> Peg for (P1, P2) {
    fn p(&self, s: &[u8]) -> Option<usize> {
        self.0.p(s).and_then(|len1|
            self.1.p(&s[len1..]).map(|len2|
                len1 + len2))
    }
}

impl<P1: Peg, P2: Peg, P3: Peg> Peg for (P1, P2, P3) {
    fn p(&self, s: &[u8]) -> Option<usize> {
        self.0.p(s).and_then(|len1|
            self.1.p(&s[len1..]).and_then(|len2|
                self.2.p(&s[len1 + len2..]).map(|len3|
                    len1 + len2 + len3)))
    }
}

macro_rules! impl_tuple {
    ( $( $p:ident $ix:ident ),* ) => {
        impl< $( $p : Peg ),* > Peg for ( $( $p ),* ) {
            fn p(&self, s: &[u8]) -> Option<usize> {
                let ( $( ref $ix ),* ) = *self;
                let mut i = 0;
                $(
                    if let Some(len) = $ix.p(&s[i..]) {
                        i += len;
                    } else {
                        return None;
                    }
                )*
                Some(i)
            }
        }
    }
}
impl_tuple!(P1 p1, P2 p2, P3 p3, P4 p4);


/// Choice from two heterogeneous alternatives.
pub struct Alt<P1, P2>(pub P1, pub P2);

impl<P1: Peg, P2: Peg> Peg for Alt<P1, P2> {
    fn p(&self, s: &[u8]) -> Option<usize> {
        self.0.p(s).or_else(|| self.1.p(s))
    }
}

/// Choice from three heterogeneous alternatives.
pub struct Alt3<P1, P2, P3>(pub P1, pub P2, pub P3);

impl<P1: Peg, P2: Peg, P3: Peg> Peg for Alt3<P1, P2, P3> {
    fn p(&self, s: &[u8]) -> Option<usize> {
        self.0.p(s).or_else(|| self.1.p(s).or_else(|| self.2.p(s)))
    }
}

/// Choice from a homogenous slice of parsers.
pub struct OneOf<'a, P: 'a>(pub &'a [P]);

impl<'a, P: Peg> Peg for OneOf<'a, P> {
    fn p(&self, s: &[u8]) -> Option<usize> {
        for ref p in self.0 {
            if let Some(len) = p.p(s) {
                return Some(len);
            }
        }
        None
    }
}

/// Repetition with a minimum and maximum (inclusive) bound
pub struct Repeat<P, R>(pub P, pub R);

impl<P: Peg> Peg for Repeat<P, usize> {
    fn p(&self, s: &[u8]) -> Option<usize> {
        let Repeat(ref p, reps) = *self;
        let mut i = 0;
        let mut count = 0;
        while count < reps {
            if let Some(len) = p.p(&s[i..]) {
                i += len;
                count += 1;
            } else {
                break;
            }
        }
        Some(i)
    }
}

impl<P: Peg> Peg for Repeat<P, ops::Range<usize>> {
    fn p(&self, s: &[u8]) -> Option<usize> {
        let Repeat(ref p, ops::Range { start, end }) = *self;
        let mut i = 0;
        let mut count = 0;
        while count + 1 < end {
            if let Some(len) = p.p(&s[i..]) {
                i += len;
                count += 1;
            } else {
                break;
            }
        }
        if count >= start {
            Some(i)
        } else {
            None
        }
    }
}

impl<P: Peg> Peg for Repeat<P, ops::RangeFrom<usize>> {
    fn p(&self, s: &[u8]) -> Option<usize> {
        let Repeat(ref p, ops::RangeFrom { start }) = *self;
        let mut i = 0;
        let mut count = 0;
        while let Some(len) = p.p(&s[i..]) {
            i += len;
            count += 1;
        }
        if count >= start {
            Some(i)
        } else {
            None
        }
    }
}

impl<P: Peg> Peg for Repeat<P, ops::RangeFull> {
    fn p(&self, s: &[u8]) -> Option<usize> {
        ZeroOrMore(Ref(&self.0)).p(s)
    }
}

impl<P: Peg> Peg for Repeat<P, ops::RangeTo<usize>> {
    fn p(&self, s: &[u8]) -> Option<usize> {
        let Repeat(ref p, ops::RangeTo { end }) = *self;
        Repeat(Ref(p), 0..end).p(s)
    }
}

pub struct Optional<P>(pub P);

impl<P: Peg> Peg for Optional<P> {
    fn p(&self, s: &[u8]) -> Option<usize> {
        self.0.p(s).or(Some(0))
    }
}

// not actually used yet
/*
pub struct OneOrMore<P>(pub P);

impl<P: Peg> Peg for OneOrMore<P> {
    fn p(&self, s: &[u8]) -> Option<usize> {
        self.0.p(s).map(|len| {
            let mut i = len;
            while let Some(len) = self.0.p(&s[i..]) {
                i += len;
            }
            i
        })
    }
}
*/

pub struct ZeroOrMore<P>(pub P);

impl<P: Peg> Peg for ZeroOrMore<P> {
    fn p(&self, s: &[u8]) -> Option<usize> {
        let mut i = 0;
        while let Some(len) = self.0.p(&s[i..]) {
            i += len;
        }
        Some(i)
    }
}

/// Fail to match if the arg matches, otherwise match empty.
pub struct FailIf<P>(pub P);

impl<P: Peg> Peg for FailIf<P> {
    fn p(&self, s: &[u8]) -> Option<usize> {
        match self.0.p(s) {
            Some(_) => None,
            None => Some(0)
        }
    }
}

/// A wrapper to use whenever you have a reference to a Peg object
pub struct Ref<'a, P: 'a>(pub &'a P);

impl<'a, P: Peg> Peg for Ref<'a, P> {
    fn p(&self, s: &[u8]) -> Option<usize> {
        self.0.p(s)
    }
}
