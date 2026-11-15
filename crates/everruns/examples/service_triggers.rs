//! Durable event triggers with a real agent. Reusing EVENT_ID is idempotent across restarts.
//! cargo run -p everruns --features openai,local,triggers --example service_triggers -- EVENT_ID
use everruns::{
    Agent, CancellationToken, Engine, Model, OpenAI,
    hosting::{AgentBuild, SqliteHosting},
    triggers::{Dispatcher, Event, Filter, Outcome, Routing, SqliteStore, Trigger},
};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_id = std::env::args()
        .nth(1)
        .ok_or("Usage: service_triggers EVENT_ID")?;
    let root = ".local/framework-triggers";
    std::fs::create_dir_all(root)?;
    let agent = Agent::builder()
        .instructions("Summarize the incoming deployment event in one sentence.")
        .model(Model::new("gpt-6-luna", OpenAI::from_env()?))
        .build()?;
    let build = AgentBuild {
        agent: "deployment-watcher".into(),
        build_id: "v1".into(),
    };
    let profile = SqliteHosting::open(format!("{root}/engine.sqlite"))?;
    let engine = Engine::builder()
        .hosting(profile.clone())
        .register(build.clone(), agent)
        .build();
    let stop = CancellationToken::new();
    let supervisor = tokio::spawn({
        let engine = engine.clone();
        let stop = stop.clone();
        async move { profile.supervise(engine, stop).await }
    });
    let dispatcher = Dispatcher::new(
        &engine,
        SqliteStore::open(format!("{root}/outbox.sqlite"))?,
        Trigger {
            key: "deployments/v1".into(),
            build,
            message: "Deployment {{payload.version}} completed for {{payload.service}}.".into(),
            filter: Filter::default(),
            routing: Routing::PerSubject,
        },
    )?;
    dispatcher.recover().await?;
    let outcome = dispatcher
        .dispatch(Event {
            id: event_id,
            source: "deployment".into(),
            kind: "completed".into(),
            subject: Some("api/production".into()),
            context: serde_json::json!({"payload":{"service":"api","version":"v1.2.3"}}),
        })
        .await?;
    match outcome {
        Outcome::Dispatched { session, input } => {
            let session = engine.resume(session).await?;
            let turn = session
                .admission(input)
                .await?
                .ok_or("Admission missing")?
                .wait()
                .await?;
            if !turn.success {
                return Err("Agent turn failed".into());
            }
            println!("{}", turn.response)
        }
        other => println!("{other:?}"),
    }
    stop.cancel();
    supervisor.await??;
    Ok(())
}
