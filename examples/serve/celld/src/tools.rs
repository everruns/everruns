use std::time::Duration;

use serde_json::Value;
use serve::prelude::*;

/// Look up a topic. Slow on purpose: `RESEARCH_DELAY_MS` (default 2000)
/// leaves time to kill the container mid-turn and watch the cell recover.
#[tool]
async fn look_up(cx: &Cx, topic: String) -> Result<Value> {
    cx.progress(format!("researching {topic}")).await;
    let delay = std::env::var("RESEARCH_DELAY_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .unwrap_or(2000);
    tokio::time::sleep(Duration::from_millis(delay)).await;
    Ok(json!({ "topic": topic, "finding": "self-hosted Durable Objects on a bucket" }))
}
