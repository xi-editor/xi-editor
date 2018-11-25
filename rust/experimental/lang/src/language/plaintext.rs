use parser::Parser;
use statestack::{Context, NewState, State};
use Scope;

lazy_static! {
    static ref PLAINTEXT_SOURCE_SCOPE: Scope = vec!["source.plaintext".to_owned()];
}

pub struct PlaintextParser<N> {
    ctx: Context<(), N>,
}

impl<N: NewState<()>> PlaintextParser<N> {
    pub fn new(new_state: N) -> PlaintextParser<N> {
        PlaintextParser { ctx: Context::new(new_state) }
    }
}

impl<N: NewState<()>> Parser for PlaintextParser<N> {

    fn get_all_scopes(&self) -> Vec<Scope> {
        vec![PLAINTEXT_SOURCE_SCOPE.clone()]
    }

    fn get_scope_for_state(&self, _state: State) -> u32 {
        0
    }

    fn parse(&mut self, text: &str, state: State) -> (usize, State, usize, State) {
        (0, self.ctx.push(state, ()), text.as_bytes().len(), state)
    }
}
