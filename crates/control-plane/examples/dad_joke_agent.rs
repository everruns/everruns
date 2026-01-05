// Example: Dad Joke Agent with Current Time capability
// Run with: cargo run --example dad_joke_agent
//
// Prerequisites:
// 1. Start the services: ./scripts/dev.sh start
// 2. Run migrations: ./scripts/dev.sh migrate
// 3. Start the API: ./scripts/dev.sh api (in another terminal)
// 4. Start the Worker: ./scripts/dev.sh worker (in another terminal)
//
// This example demonstrates:
// - Creating an agent with capabilities (current_time)
// - Creating a session and sending a message
// - Listening to SSE events until the agent responds
// - Cleaning up resources after completion

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::{BufRead, BufReader};
use std::time::Duration;

const API_BASE_URL: &str = "http://localhost:9000";
const SSE_TIMEOUT_SECS: u64 = 60;

#[derive(Debug, Deserialize)]
struct Agent {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct Session {
    id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Event {
    event_type: String,
    sequence: Option<i32>,
    session_id: String,
    data: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct ContentPart {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MessageData {
    #[allow(dead_code)]
    role: Option<String>,
    content: Option<Vec<ContentPart>>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("===========================================");
    println!("  Dad Joke Agent with Current Time");
    println!("===========================================\n");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    // Step 1: Create a Dad Joke Agent with current_time capability
    println!("Creating Dad Joke Agent...");
    let create_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "Dad Joke Agent",
            "description": "A witty agent that tells dad jokes about the current time",
            "system_prompt": "You are a master of dad jokes. When asked about time, \
                you MUST first use the get_current_time tool to get the actual current time, \
                then craft a hilarious dad joke that incorporates the real time. \
                Keep your jokes family-friendly and groan-worthy!",
            "capabilities": ["current_time"]
        }))
        .send()
        .await?;

    if !create_response.status().is_success() {
        eprintln!("Failed to create agent: {}", create_response.status());
        eprintln!("Response: {}", create_response.text().await?);
        return Ok(());
    }

    let agent: Agent = create_response.json().await?;
    println!("   Agent ID: {}", agent.id);
    println!("   Name: {}", agent.name);

    // Step 2: Create a session
    println!("\nCreating session...");
    let session_response = client
        .post(format!("{}/v1/agents/{}/sessions", API_BASE_URL, agent.id))
        .json(&json!({
            "title": "Dad Joke Time"
        }))
        .send()
        .await?;

    if !session_response.status().is_success() {
        eprintln!("Failed to create session: {}", session_response.status());
        cleanup(&client, &agent.id).await;
        return Ok(());
    }

    let session: Session = session_response.json().await?;
    println!("   Session ID: {}", session.id);

    // Step 3: Send a message asking for a dad joke about time
    println!("\nSending message: \"Tell me a joke about the current time!\"");
    let message_response = client
        .post(format!(
            "{}/v1/agents/{}/sessions/{}/messages",
            API_BASE_URL, agent.id, session.id
        ))
        .json(&json!({
            "message": {
                "role": "user",
                "content": [{
                    "type": "text",
                    "text": "Tell me a joke about the current time!"
                }]
            }
        }))
        .send()
        .await?;

    if !message_response.status().is_success() {
        eprintln!("Failed to send message: {}", message_response.status());
        cleanup(&client, &agent.id).await;
        return Ok(());
    }

    println!("   Message sent successfully!");

    // Step 4: Listen to SSE events until we get the agent's response
    println!("\nListening to SSE events...");
    println!("-------------------------------------------");

    let sse_url = format!(
        "{}/v1/agents/{}/sessions/{}/sse",
        API_BASE_URL, agent.id, session.id
    );

    // Use blocking client for SSE streaming (reqwest-eventsource alternative)
    let result = listen_to_sse(&sse_url).await;

    println!("-------------------------------------------");

    match result {
        Ok(joke) => {
            println!("\nDad Joke Agent says:");
            println!("   {}", joke);
        }
        Err(e) => {
            eprintln!("\nError listening to SSE: {}", e);
        }
    }

    // Cleanup
    cleanup(&client, &agent.id).await;

    println!("\nExample completed!");
    Ok(())
}

async fn listen_to_sse(url: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Use a blocking client in a spawn_blocking to handle SSE
    let url = url.to_string();
    let timeout = Duration::from_secs(SSE_TIMEOUT_SECS);

    let result = tokio::task::spawn_blocking(
        move || -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
            let client = reqwest::blocking::Client::builder()
                .timeout(timeout)
                .build()?;

            let response = client.get(&url).send()?;

            if !response.status().is_success() {
                return Err(format!("SSE connection failed: {}", response.status()).into());
            }

            let reader = BufReader::new(response);
            let mut current_event_type = String::new();
            let mut agent_response = String::new();
            let mut seen_tool_call = false;
            let mut seen_tool_result = false;

            for line in reader.lines() {
                let line = line?;

                // Parse SSE format
                if line.starts_with("event:") {
                    current_event_type = line.trim_start_matches("event:").trim().to_string();
                } else if line.starts_with("data:") {
                    let data = line.trim_start_matches("data:").trim();

                    if let Ok(event) = serde_json::from_str::<Event>(data) {
                        match current_event_type.as_str() {
                            "message.user" => {
                                println!("   [User Message] Received");
                            }
                            "tool.call" => {
                                if let Some(name) = event.data.get("name").and_then(|n| n.as_str())
                                {
                                    println!("   [Tool Call] {}", name);
                                    seen_tool_call = true;
                                }
                            }
                            "tool.result" => {
                                if let Some(result) =
                                    event.data.get("result").and_then(|r| r.as_str())
                                {
                                    println!("   [Tool Result] {}", result);
                                    seen_tool_result = true;
                                }
                            }
                            "message.agent" => {
                                // Parse the agent's response
                                if let Ok(msg_data) =
                                    serde_json::from_value::<MessageData>(event.data.clone())
                                {
                                    if let Some(content) = msg_data.content {
                                        for part in content {
                                            if part.content_type == "text" {
                                                if let Some(text) = part.text {
                                                    agent_response = text;
                                                }
                                            }
                                        }
                                    }
                                }

                                // If we've seen the tool flow and got the response, we're done
                                if seen_tool_call && seen_tool_result && !agent_response.is_empty()
                                {
                                    println!("   [Agent Message] Received");
                                    return Ok(agent_response);
                                }
                            }
                            "session.status" => {
                                if let Some(status) =
                                    event.data.get("status").and_then(|s| s.as_str())
                                {
                                    println!("   [Session Status] {}", status);
                                    // If session is back to pending after running, we're done
                                    if status == "pending"
                                        && seen_tool_result
                                        && !agent_response.is_empty()
                                    {
                                        return Ok(agent_response);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }

            if !agent_response.is_empty() {
                Ok(agent_response)
            } else {
                Err("No agent response received".into())
            }
        },
    )
    .await??;

    Ok(result)
}

async fn cleanup(client: &reqwest::Client, agent_id: &str) {
    println!("\nCleaning up...");
    let _ = client
        .delete(format!("{}/v1/agents/{}", API_BASE_URL, agent_id))
        .send()
        .await;
    println!("   Agent deleted");
}
