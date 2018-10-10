
use super::{Source, gen_op_seq, EngineOp};
use std::collections::BTreeSet;

use xi_rope::engine::Engine;
use xi_rope::rope::{Rope,RopeInfo};
use xi_rope::delta::Delta;


/// This is included as an example of how test cases might be written for issues
/// discovered while fuzzing. I (@cmyr) have not dug into this particular failure,
/// which happens more or less immediately after running the op_seq fuzzer.
#[test]
fn op_seq_1() {
    let inp: &[u8] = &[0x65,0x24,0x75,0xd0];
    run_op_seq_case(inp);

}

fn run_op_seq_case(inp: &[u8]) {
    let initial = Rope::from("abcd");
    let mut engine = Engine::new(initial.clone());
    let mut s = Source::new(inp);
    let seq = gen_op_seq(&mut s, initial.len()).unwrap();

    eprintln!("ops: {:?}", &seq);

    for (i, op) in seq.iter().enumerate() {
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
