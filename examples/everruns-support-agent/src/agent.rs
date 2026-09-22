use everruns::{Agent, BuildError};

use crate::tools;

pub const MODEL: &str = "claude-opus-5-5";

pub fn build(provider: impl Into<everruns::Provider>) -> Result<Agent, BuildError> {
    Agent::builder()
        .name("everruns-support-agent")
        .instructions(include_str!("resources/instructions.md"))
        .provider(provider)
        .model(MODEL)
        .max_iterations(12)
        .tool(tools::search_docs())
        .tool(tools::read_doc())
        .build()
}
