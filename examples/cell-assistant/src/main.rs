//! A cell node for real-model agents. The node brings the OpenAI provider
//! (credentials stay on the node) and a clock tool; the agents are
//! definitions clients send when they create a session.
//!
//! ```text
//! export OPENAI_API_KEY=...
//! cargo build --release -p everruns-cell-example-assistant
//! CELL_NODE_BIN=target/release/everruns-cell-example-assistant python3 scripts/cell/fleet.py up --cells 1
//! curl -s localhost:7101/v1/sessions -H 'content-type: application/json' \
//!   -d "{\"agent\": $(cat examples/cell-assistant/agents/assistant.json)}"
//! curl -s localhost:7101/v1/sessions/$ID/messages -d '{"text":"What time is it in Kyiv?"}' -H 'content-type: application/json'
//! curl -sN localhost:7102/v1/sessions/$ID/sse        # stream from any node
//! ```
//!
//! `agents/assistant.json` answers with the clock tool; `agents/reviewer.json`
//! critiques text. Both run in the same cells, and so does any other
//! definition that uses OpenAI models and this node's tools
//! (`GET /v1/tools`).

use everruns::OpenAI;
use everruns::prelude::*;
use everruns_cell::StandardHost;
use serde_json::{Value, json};

fn clock() -> FunctionTool {
    FunctionTool::new(
        "utc_now",
        "The current UTC time, RFC 3339.",
        json!({ "type": "object", "properties": {} }),
        |_args: Value| async move {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_secs();
            Ok::<_, String>(json!({ "unix_seconds": secs }))
        },
    )
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let openai = OpenAI::from_env().map_err(|error| anyhow::anyhow!("OPENAI_API_KEY: {error}"))?;
    everruns_cell::cli::run(
        StandardHost::new()
            .provider("openai", move |model| {
                Some(Model::new(model, openai.clone()))
            })
            .tool(clock),
    )
    .await
}
