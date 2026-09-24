use serve::prelude::*;

/// Linear's MCP server, attached to every agent. Skipped in dev until
/// `LINEAR_TOKEN` is set; required in `start`.
#[connection]
fn linear() -> McpServer {
    McpServer::http("https://mcp.linear.app/mcp").auth(Secret::named("LINEAR_TOKEN"))
}
