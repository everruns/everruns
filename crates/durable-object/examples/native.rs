//! Real-model native executor. The Durable Object owns admission and recovery.
use everruns::{Agent, FunctionTool, OpenAI};
use everruns_durable_object::{AuthorityClient, Executor, SessionBinding};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .name("assistant")
        .instructions("Use add to calculate sums. Be brief.")
        .provider(OpenAI::from_env()?)
        .model("gpt-6-luna")
        .tool(FunctionTool::new("add", "Add two integers", json!({"type":"object","properties":{"a":{"type":"integer"},"b":{"type":"integer"}},"required":["a","b"]}), |args: serde_json::Value| async move {
            let a = args["a"].as_i64().ok_or("missing a")?;
            let b = args["b"].as_i64().ok_or("missing b")?;
            Ok::<_,String>(json!({"sum":a.checked_add(b).ok_or("integer overflow")?}))
        }).idempotent())
        .build()?;
    let client = AuthorityClient::new(
        &std::env::var("AUTHORITY_URL")?,
        &std::env::var("TENANT_ID")?,
        &std::env::var("AUTHORITY_TOKEN")?,
    )?;
    let binding = SessionBinding {
        agent: "assistant".into(),
        build_id: std::env::var("AGENT_BUILD_ID")?,
    };
    let factory = move |id, backends| {
        let agent = agent.clone();
        async move { agent.execution_host(id, backends).await }
    };
    let executor = Executor::new(
        client,
        binding,
        &std::env::var("EXECUTOR_TOKEN")?,
        factory,
        16,
    )?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8790").await?;
    println!("Native executor listening on {}", listener.local_addr()?);
    axum::serve(listener, executor.router()).await?;
    Ok(())
}
