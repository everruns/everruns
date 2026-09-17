use everruns::{Agent, BuildError, Provider};

use crate::tools;

pub const MODEL: &str = "gpt-5.6-terra";

pub fn build(provider: impl Into<Provider>) -> Result<Agent, BuildError> {
    Agent::builder()
        .name("support-agent")
        .instructions(include_str!("resources/instructions.md"))
        .provider(provider)
        .model(MODEL)
        .max_iterations(12)
        .tool(tools::lookup_customer())
        .tool(tools::read_support_policy())
        .build()
}
