// Chat command - send message and stream response

use crate::client::Client;
use crate::output::OutputFormat;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Request to create a message
#[derive(Debug, Serialize)]
struct CreateMessageRequest {
    message: InputMessage,
}

#[derive(Debug, Serialize)]
struct InputMessage {
    role: String,
    content: Vec<InputContentPart>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum InputContentPart {
    #[serde(rename = "text")]
    Text { text: String },
}

/// Event from API
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Event {
    id: Uuid,
    #[serde(rename = "type")]
    event_type: String,
    data: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct ListResponse<T> {
    data: Vec<T>,
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    client: &Client,
    output: OutputFormat,
    quiet: bool,
    message: String,
    session_id: Uuid,
    agent_id: Option<Uuid>,
    timeout_secs: u64,
    no_stream: bool,
) -> Result<()> {
    // Resolve agent_id if not provided
    let agent_id = match agent_id {
        Some(id) => id,
        None => {
            // We need to look up the session to get agent_id
            // Try a few agent IDs or list agents first
            // For now, require --agent flag
            anyhow::bail!("--agent is required (agent_id lookup not yet implemented)");
        }
    };

    // Create the message
    let request = CreateMessageRequest {
        message: InputMessage {
            role: "user".to_string(),
            content: vec![InputContentPart::Text {
                text: message.clone(),
            }],
        },
    };

    let _: serde_json::Value = client
        .post(
            &format!("/v1/agents/{}/sessions/{}/messages", agent_id, session_id),
            &request,
        )
        .await?;

    if !quiet && output.is_text() {
        println!("You: {}\n", message);
    }

    if no_stream {
        return Ok(());
    }

    // Poll for events until turn.completed or timeout
    let start = Instant::now();
    let timeout = Duration::from_secs(timeout_secs);
    let poll_interval = Duration::from_millis(500);
    let mut last_event_id: Option<Uuid> = None;
    let mut agent_content = String::new();

    loop {
        if start.elapsed() > timeout {
            if output.is_text() {
                eprintln!("\nTimeout waiting for response");
            }
            anyhow::bail!("Timeout waiting for response");
        }

        // Build URL with since_id parameter
        let url = match last_event_id {
            Some(id) => format!(
                "/v1/agents/{}/sessions/{}/events?since_id={}",
                agent_id, session_id, id
            ),
            None => format!("/v1/agents/{}/sessions/{}/events", agent_id, session_id),
        };

        let response: ListResponse<Event> = client.get(&url).await?;

        for event in response.data {
            last_event_id = Some(event.id);

            if output.is_text() {
                // Handle message.agent events
                if event.event_type == "message.agent" {
                    // Content may be at data.content or data.message.content
                    let content = event
                        .data
                        .get("content")
                        .or_else(|| event.data.get("message").and_then(|m| m.get("content")));
                    if let Some(content) = content {
                        if let Some(parts) = content.as_array() {
                            for part in parts {
                                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                    agent_content.push_str(text);
                                }
                            }
                        }
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
                output.print_value(&event);

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
