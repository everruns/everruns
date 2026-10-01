//! Durable background-response context for the Reason LLM call (EVE-1134).

use everruns_provider::background_call::BackgroundCallContext;

impl super::ReasonAtom {
    /// Let a background provider call survive a worker restart and stop on an
    /// explicit turn cancel. The host scopes the context to this turn; the atom
    /// hands it to every LLM call it makes.
    pub fn with_background_call(mut self, context: BackgroundCallContext) -> Self {
        self.background_call = context;
        self
    }
}
