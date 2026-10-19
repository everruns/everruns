//! Read an MCP server's resources: the agent lists what the server publishes
//! with `list_mcp_resources`, then reads one with `read_mcp_resource`. Uses
//! the MCP reference server (`@modelcontextprotocol/server-everything`, run
//! with `npx`, so Node.js is required) and `gpt-6.1-sol`.
//!
//! ```text
//! cargo run -p everruns --features openai,mcp-stdio --example mcp_resources
//! ```

use everruns::{Agent, Engine, McpServer, OpenAI};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Attaching the server is the only opt-in: every turn with an MCP server
    // gets the two resource tools alongside the server's own tools.
    let everything = McpServer::stdio("everything", "npx")
        .args(["-y", "@modelcontextprotocol/server-everything"]);

    let agent = Agent::builder()
        .name("resource-reader")
        .instructions(
            "Answer from the attached MCP server's resources. List them before \
             reading, and say which resource URI each fact came from. Be terse.",
        )
        .provider(OpenAI::from_env()?)
        .model("gpt-6.1-sol")
        .mcp_server(everything)
        .build()?;

    let session = Engine::new().create(agent);
    let turn = session
        .send(
            "What resources does the everything server publish? Read the first \
             text resource and quote its content.",
        )
        .await?
        .wait()
        .await?;

    println!("response: {}", turn.response);
    println!(
        "iterations: {}, tool calls: {}",
        turn.iterations, turn.tool_calls
    );
    Ok(())
}
