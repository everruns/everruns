// Chat command - send message and stream response
//
// Uses SSE streaming with since_id for efficient event delivery.
// The snapshot-before-send pattern avoids replaying earlier turns:
//   1. Snapshot the last event ID (limit=1, efficient)
//   2. Send the message (may start a new turn or steer an existing one)
//   3. Stream events via SSE with since_id = snapshotted ID
//   4. Exit on turn.completed / turn.failed / timeout

use crate::output::OutputFormat;
use anyhow::Result;
use everruns_sdk::Everruns;
use futures::StreamExt;
use std::time::{Duration, Instant};

#[allow(clippy::too_many_arguments)]
pub async fn run(
    client: &Everruns,
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
        let opts = everruns_sdk::client::ListEventsOptions {
            limit: Some(1),
            ..Default::default()
        };
        let existing = client
            .events()
            .list_with_options(&session_id, &opts)
            .await?;
        existing.data.last().map(|e| e.id.clone())
    };

    // Create the message (may start a new turn or continue an existing one)
    client.messages().create(&session_id, &message).await?;

    if !quiet && output.is_text() {
        println!("You: {}\n", message);
    }

    if no_stream {
        return Ok(());
    }

    // Stream events via SSE with since_id for server-side filtering.
    // This replaces the old polling loop that fetched all events every 500ms.
    let mut stream_opts = everruns_sdk::sse::StreamOptions::exclude_deltas().with_max_retries(10);
    if let Some(id) = snapshot_id {
        stream_opts = stream_opts.with_since_id(id);
    }

    let mut stream = client
        .events()
        .stream_with_options(&session_id, stream_opts);

    let start = Instant::now();
    let timeout = timeout_secs.map(Duration::from_secs);
    let mut agent_content = String::new();

    loop {
        // Check timeout before waiting for next event
        if let Some(timeout) = timeout
            && start.elapsed() > timeout
        {
            stream.stop();
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
                stream.stop();
                if output.is_text() {
                    eprintln!("\nTimeout waiting for response");
                }
                anyhow::bail!("Timeout waiting for response");
            }
        };

        let event = match item {
            Ok(event) => event,
            Err(e) => {
                // SSE reconnection errors are handled internally by EventStream.
                // If we get an error here, reconnection was exhausted.
                eprintln!("Stream error: {e}");
                continue;
            }
        };

        if output.is_text() {
            // Handle output.message.completed events
            if event.event_type == "output.message.completed" {
                // Content may be at data.content or data.message.content
                let content = event
                    .data
                    .get("content")
                    .or_else(|| event.data.get("message").and_then(|m| m.get("content")));
                if let Some(content) = content
                    && let Some(parts) = content.as_array()
                {
                    for part in parts {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            agent_content.push_str(text);
                        }
                    }
                }
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
            let event_json = serde_json::json!({
                "id": event.id,
                "type": event.event_type,
                "ts": event.ts,
                "session_id": event.session_id,
                "data": event.data,
            });
            output.print_value(&event_json);

            if event.event_type == "turn.completed" {
                return Ok(());
            }

            if event.event_type == "turn.failed" {
                anyhow::bail!("Turn failed");
            }
        }
    }
}
