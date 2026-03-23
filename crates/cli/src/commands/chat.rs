// Chat command - send message and stream response

use crate::output::OutputFormat;
use anyhow::Result;
use everruns_sdk::Everruns;
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
    // Create the message
    client.messages().create(&session_id, &message).await?;

    if !quiet && output.is_text() {
        println!("You: {}\n", message);
    }

    if no_stream {
        return Ok(());
    }

    // Poll for events until turn.completed or timeout
    let start = Instant::now();
    let timeout = timeout_secs.map(Duration::from_secs);
    let poll_interval = Duration::from_millis(500);
    let mut last_event_id: Option<String> = None;
    let mut agent_content = String::new();

    loop {
        if let Some(timeout) = timeout
            && start.elapsed() > timeout
        {
            if output.is_text() {
                eprintln!("\nTimeout waiting for response");
            }
            anyhow::bail!("Timeout waiting for response");
        }

        // Fetch events via SDK (ListResponse pagination fields are optional since SDK v0.1.5)
        let response = client.events().list(&session_id).await?;

        // Filter events since last seen.
        // Use position-based lookup instead of skip_while: if last_event_id
        // is no longer in the response (server-side truncation/retention),
        // treat all returned events as new rather than silently dropping them.
        let events: Vec<_> = if let Some(ref last_id) = last_event_id {
            match response.data.iter().position(|e| &e.id == last_id) {
                Some(idx) => response.data.into_iter().skip(idx + 1).collect(),
                None => response.data, // last seen event was evicted; all events are new
            }
        } else {
            response.data
        };

        for event in events {
            last_event_id = Some(event.id.clone());

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
                // JSON/YAML output: print each event as JSON
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

        tokio::time::sleep(poll_interval).await;
    }
}
