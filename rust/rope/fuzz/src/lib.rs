extern crate xi_rope;

use xi_rope::rope::{Rope, RopeInfo};
use xi_rope::delta::{Delta, Builder};
use xi_rope::interval::{Interval};
use std::cmp::min;
use std::collections::BTreeSet;

#[cfg(test)]
mod trophies;

#[derive(Debug)]
pub enum ParseError {
    NoMoreData,
    InvalidData
}

#[derive(Debug)]
pub struct Source<'a> {
    data: &'a [u8],
    i: usize,
}

impl<'a> Source<'a> {
    pub fn new(data: &[u8]) -> Source {
        Source {data, i: 0}
    }

    pub fn gen_u8(&mut self) -> Result<u8, ParseError> {
        if self.i >= self.data.len() {
            return Err(ParseError::NoMoreData);
        }
        let num = self.data[self.i];
        self.i += 1;
        Ok(num)
    }

    pub fn gen_u8_bounded(&mut self, size: usize) -> Result<u8, ParseError> {
        let bound = min(u8::max_value() as usize, size) as u8;
        if bound == 0 {
            return Ok(0);
        // uncomment the following to get nicer cases, but slower:
        // } else if bound <= 9 {
        //     // make cases easier to read by only accepting digits for small bounds
        //     self.gen_u8().and_then(|x| {
        //         let base = '0' as u8;
        //         if x >= base && x <= (base + bound) {
        //             Ok(x - base)
        //         } else {
        //             Err(ParseError::InvalidData)
        //         }
        //     })
        } else {
            self.gen_u8().map(|x| x % bound)
        }
    }

    pub fn check_end(&mut self) -> bool {
        if self.data.len() <= self.i {
            return true;
        }
        let is_sentinel = self.data[self.i] == ('$' as u8);
        if is_sentinel {
            self.i += 1;
        }
        is_sentinel
    }

    pub fn gen_ascii_char(&mut self) -> Result<char, ParseError> {
        let c = self.gen_u8()?;
        if c >= (' ' as u8) && c <= ('z' as u8) {
            Ok(char::from(c))
        } else {
            // TODO test if accepting all bytes gives better results
            Err(ParseError::InvalidData)
        }
    }

    pub fn gen_ascii_str(&mut self) -> Result<String, ParseError> {
        let mut s = String::new();
        while !self.check_end() {
            s.push(self.gen_ascii_char()?);
        }
        Ok(s)
    }
}

pub fn gen_delta(s: &mut Source, base_len: usize) -> Result<Delta<RopeInfo>,ParseError> {
    let mut b = Builder::new(base_len);
    let mut cursor = 0;
    while !s.check_end() {
        match s.gen_ascii_char()? {
            'd' => {
                if cursor >= base_len {
                    return Err(ParseError::InvalidData);
                }
                let len = 1 + s.gen_u8_bounded(base_len-cursor-1)?;
                b.delete(Interval::new_closed_open(cursor, cursor+(len as usize)));
                cursor += len as usize;
            }
            's' => {
                if cursor >= base_len {
                    return Err(ParseError::InvalidData);
                }
                let len = 1 + s.gen_u8_bounded(base_len-cursor-1)?;
                cursor += len as usize;
            }
            'i' => {
                let ins = s.gen_ascii_str()?;
                b.replace(Interval::new_closed_open(cursor,cursor), Rope::from(ins));
            }
            _ => return Err(ParseError::InvalidData)
        }
    }
    Ok(b.build())
}

#[derive(Debug)]
pub enum EngineOp {
    Edit(Delta<RopeInfo>),
    Undo(BTreeSet<usize>),
}

pub fn gen_op_seq(s: &mut Source, base_len: usize) -> Result<Vec<EngineOp>,ParseError> {
    let mut v = Vec::new();
    let mut cur_len = base_len;
    let mut num_edits = 0;
    while !s.check_end() {
        match s.gen_ascii_char()? {
            'e' => {
                let d = gen_delta(s, cur_len)?;
                cur_len = d.new_document_len();
                v.push(EngineOp::Edit(d));
                num_edits += 1;
            }
            'u' => {
                let mut groups = BTreeSet::new();
                while num_edits > groups.len() && !s.check_end() {
                    groups.insert(s.gen_u8_bounded(num_edits)? as usize);
                }
                v.push(EngineOp::Undo(groups));
            }
            _ => return Err(ParseError::InvalidData)
        }
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xi_rope::rope::{Rope};

    #[test]
    fn test_gen_delta() {
        let mut s = Source::new("iabc$".as_bytes());
        let d = gen_delta(&mut s, 4).unwrap();
        let res = String::from(d.apply(&Rope::from("1234")));
        // println!("{:?}", d);
        assert_eq!("abc1234", res);
    }

    #[test]
    fn test_gen_op_seq() {
        let mut s = Source::new("eiabc$$es\x01d\x01iz$$u\x00$".as_bytes());
        let seq = gen_op_seq(&mut s, 4).unwrap();

        let mut r = Rope::from("1234");
        for op in &seq {
            if let &EngineOp::Edit(ref d) = op {
                r = d.apply(&r);
            }
        }
        // println!("{:?}", d);
        assert_eq!("abz234", String::from(&r));
    }
}
