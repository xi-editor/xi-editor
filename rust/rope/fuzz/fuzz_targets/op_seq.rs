#![no_main]
#[macro_use] extern crate libfuzzer_sys;
extern crate xi_rope;
extern crate xi_rope_fuzz;

use xi_rope_fuzz::{Source, gen_op_seq, EngineOp};
use std::collections::BTreeSet;

use xi_rope::engine::Engine;
use xi_rope::rope::{Rope,RopeInfo};
use xi_rope::delta::Delta;

fuzz_target!(|data: &[u8]| {
    let mut s = Source::new(data);
    let initial = Rope::from("abcd");
    let mut engine = Engine::new(initial.clone());
    if let Ok(seq) = gen_op_seq(&mut s, initial.len()) {
        println!("{:?}", seq);

        for (i,op) in seq.iter().enumerate() {
            let head_rev = engine.get_head_rev_id().token();
            match *op {
                EngineOp::Edit(ref d) => engine.edit_rev(0,i,head_rev,d.clone()),
                EngineOp::Undo(ref groups) => engine.undo(groups.clone())
            }
        }

        let empty_set = BTreeSet::new();
        let latest_undo = seq.iter().rev().filter_map(|op| {
            if let EngineOp::Undo(ref groups) = *op { Some(groups) } else { None }
        }).next().unwrap_or(&empty_set);
        let raw_edits : Vec<Delta<RopeInfo>> = seq.iter().enumerate().filter_map(|(i,op)| {
            match *op {
                EngineOp::Edit(ref d) if !latest_undo.contains(&i) => Some(d.clone()),
                _ => None,
            }
        }).collect();

        let mut content = initial.clone();
        for d in &raw_edits {
            content = d.apply(&content);
        }

        assert_eq!(String::from(&content), String::from(engine.get_head()));
    }
});
