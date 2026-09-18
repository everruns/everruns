use everruns::{Agent, BuildError, Jev, Provider};

use crate::tools;

pub const MODEL: &str = "claude-sonnet-5";

pub fn build(provider: impl Into<Provider>, jev: Jev) -> Result<Agent, BuildError> {
    Agent::builder()
        .name("open-mic-agent")
        .instructions(include_str!("resources/instructions.md"))
        .provider(provider)
        .model(MODEL)
        .max_iterations(12)
        .tool(tools::read_submission())
        .tool(tools::read_house_rules())
        // The classifier the agent measures with. It writes its own questions;
        // the house rules say which ones and what each threshold buys.
        .capability(jev)
        .build()
}
