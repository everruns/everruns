//! OAuth login and an authenticated real MCP tool call.
//! cargo run -p everruns --features mcp-auth,openai --example mcp_oauth -- https://MCP_HOST/mcp http://localhost:8080/callback
//! Register/allow the redirect URI on your authorization server first.
use everruns::{
    Agent, Engine, McpServer, Model, OpenAI,
    mcp::auth::{McpAuth, MemoryTokenStore, OAuthLogin, TokenStore},
};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("Usage: mcp_oauth MCP_URL REDIRECT_URI".into());
    }
    let login = OAuthLogin::begin(&args[0], &args[1], "Everruns Framework example").await?;
    println!(
        "Open this login URL in your browser:\n{}\n\nAfter authorization, paste the complete callback URL (kept in this process only):",
        login.authorization_url()
    );
    let callback = tokio::task::spawn_blocking(|| {
        let mut callback = String::new();
        std::io::stdin().read_line(&mut callback).map(|_| callback)
    })
    .await??;
    let callback = callback.trim();
    let (_, query) = callback
        .split_once('?')
        .ok_or("Callback URL has no query")?;
    // Parse through the SDK's existing URL type without publishing credentials in Debug.
    let values = everruns::mcp::auth::parse_callback_query(query)?;
    let tokens = login
        .complete(&values.code, &values.state, values.issuer.as_deref())
        .await?;
    let store = MemoryTokenStore::default();
    store.save("service", &tokens).await?;
    let agent = Agent::builder()
        .instructions(concat!(
            "Use the attached MCP server's tools to answer the request. ",
            "Never ask for credentials in chat.",
        ))
        .model(Model::new("gpt-6.1-sol", OpenAI::from_env()?))
        .mcp_server(McpServer::http("service", &args[0]))
        .mcp_auth(McpAuth::new().oauth("service", &args[0], store))
        .build()?;
    let turn = Engine::new()
        .create(agent)
        .run("Discover the available tools, use a safe read-only one, and summarize its result.")
        .await?;
    if !turn.success {
        return Err(format!("Agent failed: {:?}", turn.error).into());
    }
    println!("{}", turn.response);
    Ok(())
}
