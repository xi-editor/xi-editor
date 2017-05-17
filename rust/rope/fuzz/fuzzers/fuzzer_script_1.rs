#![no_main]
#[macro_use] extern crate libfuzzer_sys;
extern crate xi_rope;
extern crate xi_rope_fuzz;

use xi_rope_fuzz::{Source, gen_delta};

use xi_rope::engine::Engine;
use xi_rope::rope::Rope;

fuzz_target!(|data: &[u8]| {
    let mut s = Source::new(data);
    let initial = Rope::from("abcd");
    let mut engine = Engine::new(initial.clone());
    if let Ok(d) = gen_delta(&mut s, initial.len()) {
        let raw_apply = d.apply(&initial);
        engine.edit_rev(0, 0, 0, d);
        assert_eq!(String::from(raw_apply), String::from(engine.get_head()));
    }
});
