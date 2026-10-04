//! writer: an `everruns` agent that delegates research to the `researcher`
//! agent over A2A, then drafts from its notes.
//!
//! Start the researcher first (`cargo run -p serve-example-a2a --bin
//! researcher`), then:
//!
//! ```sh
//! cargo run -p serve-example-a2a --bin writer -- "tide pools"
//! ```
//!
//! Decisions:
//! - Delegation is the built-in `a2a_agent_delegation` capability, configured
//!   with the researcher's base URL. It resolves the Agent Card under it and
//!   calls `SendMessage`; the model sees one `spawn_agent` tool.
//! - Offline by default: without `OPENAI_API_KEY` the writer's model is a
//!   script that calls `spawn_agent` with the topic and then replies. The
//!   researcher's answer is real either way: it comes over A2A.

use everruns::providers::openai::OpenAI;
use everruns::{
    Agent, CapabilityRef, Engine, LlmSimConfig, LocalConfig, Model, OnExhausted, SimToolCall,
    SimTurn,
};
use everruns_example_demo as demo;
use serde_json::{Value, json};

const MODEL: &str = "gpt-5.6-terra";
const RESEARCHER: &str = "http://127.0.0.1:3000/v1/channels/researcher/a2a";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let topic = if input.is_empty() {
        "tide pools"
    } else {
        &input
    };
    let researcher = std::env::var("RESEARCHER_URL").unwrap_or_else(|_| RESEARCHER.to_string());

    let agent = Agent::builder()
        .name("writer")
        .instructions(
            "You write short explainers. Before writing, delegate research on the topic to \
             the `researcher` agent with `spawn_agent` (target type `external_a2a`, id \
             `researcher`, mode `foreground`). Then write one paragraph from its notes only.",
        )
        .model(model(topic))
        .capability(CapabilityRef::new("a2a_agent_delegation").config(delegation(&researcher)))
        // Delegated runs are recorded in session storage, which a local
        // (SQLite-backed) session provides.
        .local(LocalConfig::new(
            std::env::temp_dir().join("everruns-a2a-writer"),
        ))
        .build()?;

    println!("RESEARCHER: {researcher}");
    let session = Engine::new().create(agent);
    demo::run(&session, &format!("Write a short explainer on {topic}.")).await?;
    Ok(())
}

/// One external agent, `researcher`, found by its Agent Card under `url`.
fn delegation(url: &str) -> Value {
    json!({
        "agents": [{
            "id": "researcher",
            "name": "Researcher",
            "description": "Researches a topic and answers with short factual notes.",
            "base_url": url,
            "preferred_binding": "JSONRPC",
            // The researcher runs on this machine; remote URLs need no flag.
            "allow_local_urls": true,
        }]
    })
}

/// OpenAI when `OPENAI_API_KEY` is set; otherwise a script that delegates
/// `topic` and then replies.
fn model(topic: &str) -> Model {
    if let Ok(key) = std::env::var("OPENAI_API_KEY")
        && !key.is_empty()
    {
        return Model::new(MODEL, OpenAI::new(key));
    }
    let delegate = SimTurn::ToolCalls(vec![SimToolCall {
        name: "spawn_agent".into(),
        arguments: json!({
            "name": "research",
            "instructions": format!("Research {topic} for a short explainer."),
            "target": { "type": "external_a2a", "id": "researcher" },
            "mode": "foreground",
            "wait_timeout_secs": 60,
        }),
        id: None,
    }]);
    let reply = SimTurn::Assistant(
        "(Offline demo reply: the writer's simulator does not read the notes; the \
         researcher's answer is the spawn_agent result above. Set OPENAI_API_KEY to draft \
         from it.)"
            .into(),
    );
    Model::simulated_with_config(
        LlmSimConfig::scripted(vec![delegate, reply]).with_on_exhausted(OnExhausted::Loop),
    )
}
