//! The agent declaration: Bashkit, Shell Code Mode in CLI mode, one MCP server.

use everruns::{Agent, BashkitShell, BuildError, CapabilityRef, McpServer, Provider};
use serde_json::json;

use crate::acme;

/// `gpt-6.2-sol`, through OpenRouter when `OPENROUTER_API_KEY` is set and
/// straight from OpenAI otherwise.
pub fn provider() -> Result<(Provider, &'static str), Box<dyn std::error::Error>> {
    if std::env::var_os("OPENROUTER_API_KEY").is_some() {
        let openrouter = everruns::providers::openrouter::OpenRouter::from_env()?;
        return Ok((openrouter.into(), "openai/gpt-6.2-sol"));
    }
    Ok((everruns::OpenAI::from_env()?.into(), "gpt-6.2-sol"))
}

/// `server_command` is the program that serves the Acme MCP server over stdio.
pub fn build(provider: Provider, model: &str, server_command: &str) -> Result<Agent, BuildError> {
    let acme = McpServer::stdio(acme::SERVER, server_command)
        .arg("--mcp-server")
        .description("Acme Corp internal APIs: CRM, billing, inventory, HR and 21 more services");

    Agent::builder()
        .name("acme-analyst")
        .instructions(include_str!("resources/instructions.md"))
        .provider(provider)
        .model(model)
        .max_iterations(12)
        .capability(BashkitShell::new())
        // CLI mode moves every MCP tool off the model's tool list and onto the
        // `tools` command inside the shell. The server labels each tool with
        // read-only or destructive hints, so the decision service is not
        // needed to rate unlabeled ones.
        .capability(
            CapabilityRef::new("shell_code_mode")
                .config(json!({"mode": "cli", "rate_unlabeled_tools": false})),
        )
        .mcp_server(acme)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_without_contacting_the_provider_or_the_server() {
        let provider: Provider = everruns::OpenAI::new("test-key").into();
        assert!(build(provider, "gpt-6.2-sol", "/nonexistent/acme").is_ok());
    }
}
