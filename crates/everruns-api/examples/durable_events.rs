// Example: Durable event streaming with UUID7-based resumption
// Run with: cargo run --example durable_events
//
// Prerequisites:
// 1. Start the services: ./scripts/dev.sh start
// 2. Run migrations: ./scripts/dev.sh migrate
// 3. Start the API: ./scripts/dev.sh api (in another terminal)
// 4. Start the worker: ./scripts/dev.sh worker (in another terminal)
//
// This example demonstrates the durable streams pattern with UUID7:
// - Fetching events with UUID7-based pagination (offset + limit)
// - Using next_offset (UUID7) for continuation
// - Saving and resuming from checkpoints
//
// Why UUID7?
// - Time-ordered: First 48 bits are Unix timestamp in ms
// - Already stored as event ID - no separate sequence needed
// - Globally unique across sessions

use serde::{Deserialize, Serialize};
use serde_json::json;

const API_BASE_URL: &str = "http://localhost:9000";

/// Response from the events endpoint with pagination info
#[derive(Debug, Deserialize)]
struct EventsResponse {
    data: Vec<Event>,
    next_offset: Option<String>, // UUID7 string
    has_more: bool,
}

/// Single event from the stream
#[derive(Debug, Deserialize)]
struct Event {
    id: String, // UUID7
    #[allow(dead_code)]
    sequence: i32, // Legacy field, kept for API compatibility
    event_type: String,
    data: serde_json::Value,
}

/// Agent response
#[derive(Debug, Deserialize)]
struct Agent {
    id: String,
}

/// Session response
#[derive(Debug, Deserialize)]
struct Session {
    id: String,
}

/// Message response
#[derive(Debug, Deserialize)]
struct Message {
    id: String,
}

/// Checkpoint state that would be saved to persistent storage
#[derive(Debug, Serialize, Deserialize)]
struct Checkpoint {
    session_id: String,
    last_offset: Option<String>, // UUID7 string
}

impl Checkpoint {
    fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            last_offset: None,
        }
    }

    fn update(&mut self, offset: &str) {
        self.last_offset = Some(offset.to_string());
        println!("   💾 Saved checkpoint: offset={}...", &offset[..8]);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();

    println!("=== Durable Streams Example (UUID7) ===\n");

    // Step 1: Create an agent
    println!("1. Creating agent...");
    let agent: Agent = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "Durable Streams Demo Agent",
            "description": "Demonstrates UUID7-based event streaming",
            "system_prompt": "You are a helpful assistant. Keep responses brief."
        }))
        .send()
        .await?
        .json()
        .await?;
    println!("   Created agent: {}", agent.id);

    // Step 2: Create a session
    println!("\n2. Creating session...");
    let session: Session = client
        .post(format!("{}/v1/agents/{}/sessions", API_BASE_URL, agent.id))
        .json(&json!({}))
        .send()
        .await?
        .json()
        .await?;
    println!("   Created session: {}", session.id);

    // Initialize checkpoint (in real app, load from persistent storage)
    let mut checkpoint = Checkpoint::new(&session.id);

    // Step 3: Send first message
    println!("\n3. Sending first message...");
    let message1: Message = client
        .post(format!(
            "{}/v1/agents/{}/sessions/{}/messages",
            API_BASE_URL, agent.id, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Hello! What is 2+2?"}]
            }
        }))
        .send()
        .await?
        .json()
        .await?;
    println!("   Sent message (id={})", message1.id);

    // Wait for processing
    println!("   Waiting for response...");
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Step 4: Fetch events from beginning (no offset)
    println!("\n4. Fetching events from beginning...");
    let events = fetch_events(&client, &agent.id, &session.id, None, 10).await?;
    print_events(&events);

    // Save checkpoint at current position
    if let Some(ref offset) = events.next_offset {
        checkpoint.update(offset);
    }

    // Step 5: Send second message
    println!("\n5. Sending second message...");
    let message2: Message = client
        .post(format!(
            "{}/v1/agents/{}/sessions/{}/messages",
            API_BASE_URL, agent.id, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Now what is 3+3?"}]
            }
        }))
        .send()
        .await?
        .json()
        .await?;
    println!("   Sent message (id={})", message2.id);

    // Wait for processing
    println!("   Waiting for response...");
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Step 6: Resume from checkpoint (only fetch new events)
    println!(
        "\n6. Resuming from checkpoint (offset={}...)...",
        checkpoint
            .last_offset
            .as_ref()
            .map(|s| &s[..8])
            .unwrap_or("none")
    );
    let new_events = fetch_events(
        &client,
        &agent.id,
        &session.id,
        checkpoint.last_offset.as_deref(),
        10,
    )
    .await?;
    print_events(&new_events);

    // Update checkpoint with new position
    if let Some(ref offset) = new_events.next_offset {
        checkpoint.update(offset);
    }

    // Step 7: Demonstrate pagination with small limit
    println!("\n7. Demonstrating pagination (limit=2)...");
    let mut offset: Option<String> = None;
    let mut page = 1;
    loop {
        let response = fetch_events(&client, &agent.id, &session.id, offset.as_deref(), 2).await?;
        println!(
            "   Page {}: {} events, has_more={}",
            page,
            response.data.len(),
            response.has_more
        );

        if !response.has_more {
            break;
        }

        offset = response.next_offset;
        page += 1;

        if page > 10 {
            println!("   (stopping after 10 pages)");
            break;
        }
    }

    // Step 8: Check for checkpoint events
    println!("\n8. Checking for checkpoint events...");
    let all_events = fetch_events(&client, &agent.id, &session.id, None, 100).await?;
    let checkpoint_events: Vec<_> = all_events
        .data
        .iter()
        .filter(|e| e.event_type == "checkpoint")
        .collect();
    println!("   Found {} checkpoint events:", checkpoint_events.len());
    for event in checkpoint_events {
        let status = event
            .data
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let last_id = event
            .data
            .get("last_event_id")
            .and_then(|v| v.as_str())
            .map(|s| &s[..8])
            .unwrap_or("?");
        println!(
            "   - id={}...: status={}, last_id={}...",
            &event.id[..8],
            status,
            last_id
        );
    }

    println!("\n=== Example Complete ===");
    println!("\nKey takeaways:");
    println!("1. Use ?offset=UUID7 to resume from a known position");
    println!("2. Use ?limit=M to paginate large event streams");
    println!("3. Save next_offset (UUID7) from responses for resumption");
    println!("4. Checkpoint events contain last_event_id for safe resumption");

    Ok(())
}

/// Fetch events with UUID7-based pagination
async fn fetch_events(
    client: &reqwest::Client,
    agent_id: &str,
    session_id: &str,
    offset: Option<&str>,
    limit: i32,
) -> Result<EventsResponse, Box<dyn std::error::Error>> {
    let mut url = format!(
        "{}/v1/agents/{}/sessions/{}/events?limit={}",
        API_BASE_URL, agent_id, session_id, limit
    );

    if let Some(off) = offset {
        url.push_str(&format!("&offset={}", off));
    }

    let response = client.get(&url).send().await?;

    // Check for Cache-Control header (indicates historical vs live data)
    if let Some(cache_control) = response.headers().get("cache-control") {
        if cache_control.to_str().unwrap_or("").contains("immutable") {
            println!("   📦 Cache-Control: immutable (historical data)");
        }
    }

    Ok(response.json().await?)
}

/// Print events in a readable format
fn print_events(response: &EventsResponse) {
    println!(
        "   Received {} events (next_offset={}, has_more={})",
        response.data.len(),
        response
            .next_offset
            .as_ref()
            .map(|s| format!("{}...", &s[..8]))
            .unwrap_or_else(|| "none".to_string()),
        response.has_more
    );
    for event in &response.data {
        let preview = match &event.data {
            serde_json::Value::Object(obj) => {
                if let Some(content) = obj.get("content") {
                    format!("{:.50}...", content.to_string())
                } else if let Some(status) = obj.get("status") {
                    format!("status={}", status)
                } else {
                    format!("{:.30}...", event.data.to_string())
                }
            }
            _ => format!("{:.30}...", event.data.to_string()),
        };
        println!(
            "   - [{}...] {}: {}",
            &event.id[..8],
            event.event_type,
            preview
        );
    }
}
