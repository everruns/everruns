// Chat command - send message and stream response
//
// Uses SSE streaming with since_id for efficient event delivery.
// The snapshot-before-send pattern avoids replaying earlier turns:
//   1. Snapshot the last event ID (limit=1, efficient)
//   2. Send the message (may start a new turn or steer an existing one)
//   3. Stream events via SSE with since_id = snapshotted ID
//   4. Exit on turn.completed / turn.failed / timeout

use crate::commands::api::ApiClient;
use crate::contract;
use crate::events::{DELTA_EVENTS, EventStream};
use crate::output::OutputFormat;
use anyhow::Result;
use serde_json::json;
use std::time::{Duration, Instant};

#[allow(clippy::too_many_arguments)]
pub async fn run(
    client: ApiClient<'_>,
    output: OutputFormat,
    quiet: bool,
    message: String,
    session_id: String,
    timeout_secs: Option<u64>,
    no_stream: bool,
) -> Result<()> {
    // Snapshot the last event ID *before* sending the message so the SSE
    // stream only sees events produced after this point — whether the
    // message starts a new turn or steers an existing one.
    let snapshot_id: Option<String> = if no_stream {
        None
    } else {
        let latest = contract::execute(
            &client,
            "list_events",
            json!({ "session_id": session_id, "limit": 1, "order_desc": true }),
        )
        .await?;
        latest["data"][0]["id"].as_str().map(ToOwned::to_owned)
    };

    // Create the message (may start a new turn or continue an existing one)
    contract::execute(
        &client,
        "create_message",
        json!({
            "session_id": session_id,
            "message": { "role": "user", "content": [{ "type": "text", "text": message }] },
        }),
    )
    .await?;

    if !quiet && output.is_text() {
        println!("You: {}\n", message);
    }

    if no_stream {
        return Ok(());
    }

    // Stream events via SSE with since_id for server-side filtering.
    let mut stream = EventStream::new(client, &session_id, snapshot_id)
        .excluding(DELTA_EVENTS)
        .with_max_retries(10);

    let start = Instant::now();
    let timeout = timeout_secs.map(Duration::from_secs);
    let mut agent_content = String::new();

    loop {
        // Check timeout before waiting for next event
        if let Some(timeout) = timeout
            && start.elapsed() > timeout
        {
            if output.is_text() {
                eprintln!("\nTimeout waiting for response");
            }
            anyhow::bail!("Timeout waiting for response");
        }

        let next = if let Some(t) = timeout {
            let remaining = t.saturating_sub(start.elapsed());
            tokio::time::timeout(remaining, stream.next()).await
        } else {
            Ok(stream.next().await)
        };

        let item = match next {
            Ok(Some(item)) => item,
            Ok(None) => {
                // Stream ended without turn completion
                if output.is_text() && !agent_content.is_empty() {
                    println!("Agent: {}", agent_content);
                }
                anyhow::bail!("Event stream ended before turn completed");
            }
            Err(_) => {
                // Timeout
                if output.is_text() {
                    eprintln!("\nTimeout waiting for response");
                }
                anyhow::bail!("Timeout waiting for response");
            }
        };

        let event = match item {
            Ok(event) => event,
            Err(e) => {
                // Reconnection is handled inside EventStream; an error here
                // means its retries are spent.
                anyhow::bail!("Event stream failed: {e}");
            }
        };

        if output.is_text() {
            // Handle output.message.completed events
            if event.event_type == "output.message.completed"
                && let Ok(data) = serde_json::from_value::<
                    everruns_core::events::OutputMessageCompletedData,
                >(event.data.clone())
                && let Some(text) = everruns_core::conversation::said_text(&data.message)
            {
                // Only what the agent said, never its commentary.
                if !agent_content.is_empty() {
                    agent_content.push_str("\n\n");
                }
                agent_content.push_str(&text);
            }

            // Handle tool.progress event
            if event.event_type == "tool.progress"
                && let Some(message) = event.data.get("message").and_then(|m| m.as_str())
            {
                let tool = event
                    .data
                    .get("display_name")
                    .or_else(|| event.data.get("tool_name"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("tool");
                eprintln!("  [{tool}] {message}");
            }

            // Handle tool.output.delta event (streamed tool output)
            if event.event_type == "tool.output.delta"
                && let Some(delta) = event.data.get("delta").and_then(|d| d.as_str())
            {
                let stream_name = event
                    .data
                    .get("stream")
                    .and_then(|s| s.as_str())
                    .unwrap_or("stdout");
                let tool = event
                    .data
                    .get("tool_name")
                    .and_then(|t| t.as_str())
                    .unwrap_or("tool");
                let trimmed = delta.trim_end_matches('\n');
                if stream_name == "stderr" {
                    eprintln!("  [{tool}:stderr] {trimmed}");
                } else {
                    eprintln!("  [{tool}] {trimmed}");
                }
            }

            // Handle turn.completed event
            if event.event_type == "turn.completed" {
                if !agent_content.is_empty() {
                    println!("Agent: {}", agent_content);
                }
                return Ok(());
            }

            // Handle turn.failed event
            if event.event_type == "turn.failed" {
                let error = event
                    .data
                    .get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("Unknown error");
                eprintln!("\nTurn failed: {}", error);
                anyhow::bail!("Turn failed: {}", error);
            }
        } else {
            // JSON/YAML output: print each event
            output.print_value(&event.to_json());

            if event.event_type == "turn.completed" {
                return Ok(());
            }

            if event.event_type == "turn.failed" {
                anyhow::bail!("Turn failed");
            }
        }
    }
}
