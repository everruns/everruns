//! Real-model native executor. The Durable Object owns admission and recovery.
use everruns::{Engine, Model, OpenAI, hosting::AgentBuild};
use everruns_durable_object::{AuthorityClient, Executor, ObjectHosting, SessionBinding};

#[path = "../../../examples/durable-object-agent/src/agent.rs"]
mod agent;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = agent::counter(Model::new("gpt-6-luna", OpenAI::from_env()?))?;
    let client = AuthorityClient::new(
        &std::env::var("AUTHORITY_URL")?,
        &std::env::var("TENANT_ID")?,
        &std::env::var("AUTHORITY_TOKEN")?,
    )?;
    let binding = SessionBinding {
        agent: "counter".into(),
        build_id: std::env::var("AGENT_BUILD_ID")?,
    };
    let engine = Engine::builder()
        .hosting(ObjectHosting::new(client.clone()))
        .register(AgentBuild::new(&binding.agent, &binding.build_id), agent)
        .build();
    let executor = Executor::new(
        client,
        binding,
        &std::env::var("EXECUTOR_TOKEN")?,
        engine,
        16,
    )?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8790").await?;
    println!("Native executor listening on {}", listener.local_addr()?);
    axum::serve(listener, executor.router()).await?;
    Ok(())
}
