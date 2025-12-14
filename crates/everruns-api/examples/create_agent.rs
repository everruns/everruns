// Example: Create an agent via the API
// Run with: cargo run --example create_agent
//
// Prerequisites:
// 1. Start the services: ./scripts/dev.sh start
// 2. Run migrations: ./scripts/dev.sh migrate
// 3. Start the API: ./scripts/dev.sh api (in another terminal)

use everruns_contracts::Agent;
use serde_json::json;

const API_BASE_URL: &str = "http://localhost:9000";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();

    // Step 1: Create an agent
    println!("📝 Creating agent...");
    let create_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "My First Agent",
            "description": "A helpful AI assistant",
            "default_model_id": "gpt-5.1",
            "definition": {
                "system_prompt": "You are a helpful AI assistant. Be concise and friendly.",
                "temperature": 0.7,
                "max_tokens": 2000,
                "tools": []
            }
        }))
        .send()
        .await?;

    if !create_response.status().is_success() {
        eprintln!("❌ Failed to create agent: {}", create_response.status());
        eprintln!("Response: {}", create_response.text().await?);
        return Ok(());
    }

    let agent: Agent = create_response.json().await?;
    println!("✅ Created agent:");
    println!("   ID: {}", agent.id);
    println!("   Name: {}", agent.name);
    println!("   Status: {:?}", agent.status);
    println!("   Created at: {}", agent.created_at);
    println!(
        "   Definition: {}",
        serde_json::to_string_pretty(&agent.definition)?
    );

    // Step 2: Retrieve the agent
    println!("\n🔍 Retrieving agent...");
    let get_response = client
        .get(format!("{}/v1/agents/{}", API_BASE_URL, agent.id))
        .send()
        .await?;

    let retrieved_agent: Agent = get_response.json().await?;
    println!("✅ Retrieved agent:");
    println!("   ID: {}", retrieved_agent.id);
    println!("   Name: {}", retrieved_agent.name);
    println!(
        "   Description: {}",
        retrieved_agent.description.unwrap_or_default()
    );

    // Step 3: List all agents
    println!("\n📋 Listing all agents...");
    let list_response = client
        .get(format!("{}/v1/agents", API_BASE_URL))
        .send()
        .await?;

    let agents: Vec<Agent> = list_response.json().await?;
    println!("✅ Found {} agent(s):", agents.len());
    for a in agents {
        println!("   - {} ({})", a.name, a.id);
    }

    println!("\n🎉 Example completed successfully!");
    println!("\n💡 Next steps:");
    println!("   - Create a thread: POST /v1/threads");
    println!("   - Add messages: POST /v1/threads/<thread_id>/messages");
    println!("   - Create a run: POST /v1/runs");
    println!("   - View API docs: http://localhost:9000/swagger-ui/");

    Ok(())
}
