use std::collections::HashMap;

use rust::StateEl;

#[derive(Default)]
pub struct ElementTracker {
    elements: HashMap<StateEl, u32>,
    next_id: u32
}

impl ElementTracker {
    pub fn lookup(&mut self, element: &StateEl) -> LookupResult {
        if let Some(id) = self.elements.get(element) {
            return LookupResult::Existing(*id);
        }

        let old_id = self.next_id;
        self.next_id += 1;

        self.elements.insert(element.clone(), old_id);
        LookupResult::New(old_id)
    }
}

pub enum LookupResult {
    Existing(u32),
    New(u32)
}