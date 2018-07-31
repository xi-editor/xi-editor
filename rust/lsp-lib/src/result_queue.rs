use std::collections::VecDeque;
use types::{LspResponse, LanguageResponseError};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Default)]
pub struct ResultQueue(Arc<Mutex<VecDeque<(usize, Result<LspResponse, LanguageResponseError>)>>>);

impl ResultQueue {

    pub fn new() -> Self {
        ResultQueue(Arc::new(Mutex::new(VecDeque::new())))
    }

    pub fn push_result(&mut self, request_id: usize, response: Result<LspResponse, LanguageResponseError>) {
        let mut queue = self.0.lock().unwrap();
        queue.push_back((request_id, response));
    }

    pub fn pop_result(&mut self) -> Option<(usize, Result<LspResponse, LanguageResponseError>)> {
        let mut queue = self.0.lock().unwrap();
        queue.pop_front()
    }
}