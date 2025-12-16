// Example: Create a harness via the API (M2)
// Run with: cargo run --example create_harness
//
// Prerequisites:
// 1. Start the services: ./scripts/dev.sh start
// 2. Run migrations: ./scripts/dev.sh migrate
// 3. Start the API: ./scripts/dev.sh api (in another terminal)

use everruns_contracts::Harness;
use serde_json::json;

const API_BASE_URL: &str = "http://localhost:9000";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();

    // Step 1: Create a harness
    println!("Creating harness...");
    let create_response = client
        .post(format!("{}/v1/harnesses", API_BASE_URL))
        .json(&json!({
            "slug": "my-first-harness",
            "display_name": "My First Harness",
            "description": "A helpful AI assistant",
            "system_prompt": "You are a helpful AI assistant. Be concise and friendly.",
            "temperature": 0.7,
            "max_tokens": 2000
        }))
        .send()
        .await?;

    if !create_response.status().is_success() {
        eprintln!("Failed to create harness: {}", create_response.status());
        eprintln!("Response: {}", create_response.text().await?);
        return Ok(());
    }

    let harness: Harness = create_response.json().await?;
    println!("Created harness:");
    println!("   ID: {}", harness.id);
    println!("   Slug: {}", harness.slug);
    println!("   Name: {}", harness.display_name);
    println!("   Status: {:?}", harness.status);
    println!("   Created at: {}", harness.created_at);

    // Step 2: Retrieve the harness
    println!("\nRetrieving harness...");
    let get_response = client
        .get(format!("{}/v1/harnesses/{}", API_BASE_URL, harness.id))
        .send()
        .await?;

    let retrieved_harness: Harness = get_response.json().await?;
    println!("Retrieved harness:");
    println!("   ID: {}", retrieved_harness.id);
    println!("   Name: {}", retrieved_harness.display_name);
    println!(
        "   Description: {}",
        retrieved_harness.description.unwrap_or_default()
    );

    // Step 3: List all harnesses
    println!("\nListing all harnesses...");
    let list_response = client
        .get(format!("{}/v1/harnesses", API_BASE_URL))
        .send()
        .await?;

    let response: serde_json::Value = list_response.json().await?;
    let harnesses: Vec<Harness> = serde_json::from_value(response["data"].clone())?;
    println!("Found {} harness(es):", harnesses.len());
    for h in harnesses {
        println!("   - {} ({})", h.display_name, h.id);
    }

    println!("\nExample completed successfully!");
    println!("\nNext steps:");
    println!("   - Create a session: POST /v1/harnesses/<harness_id>/sessions");
    println!("   - Send a message: POST /v1/harnesses/<harness_id>/sessions/<session_id>/events");
    println!("   - View API docs: http://localhost:9000/swagger-ui/");

    Ok(())
}
