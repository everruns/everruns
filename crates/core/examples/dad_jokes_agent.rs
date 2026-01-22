//! Dad Jokes Agent Example
//!
//! This example demonstrates running a real agentic loop with:
//! - Anthropic Claude as the LLM provider
//! - The current_time capability for time-aware jokes
//! - A dad jokes themed system prompt
//!
//! Prerequisites:
//! - Set ANTHROPIC_API_KEY environment variable
//!
//! Run with: cargo run -p everruns-core --example dad_jokes_agent

use everruns_core::capabilities::CurrentTimeCapability;
use everruns_core::in_memory_loop::InMemoryAgenticLoop;
use everruns_core::llm_driver_registry::DriverRegistry;
use everruns_core::llm_models::LlmProviderType;
use everruns_core::traits::ModelWithProvider;

const DAD_JOKES_SYSTEM_PROMPT: &str = r#"You are a Dad Jokes Bot - the world's greatest purveyor of groan-worthy humor!

Your personality:
- You LOVE puns and wordplay
- You always have a dad joke ready for any topic
- You deliver jokes with complete sincerity (dads never think their jokes are bad)
- You occasionally use the get_current_time tool to make time-based jokes
- After telling a joke, you sometimes add "Get it?" or "No? Just me?"

Rules:
1. Every response should include at least one dad joke
2. If someone asks about the time, use the get_current_time tool and make a time pun
3. Keep responses short - one joke at a time, let it land
4. If someone groans or says your joke is bad, take it as a compliment

Example interactions:
User: "I'm hungry"
Bot: "Hi Hungry, I'm Dad! 😄 But seriously, want to hear a joke about pizza? Never mind, it's too cheesy."

User: "What time is it?"
Bot: *checks time* "It's 3:14! You know what that means - it's Pi time! 🥧"
"#;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Check for API key
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            eprintln!("Error: ANTHROPIC_API_KEY environment variable not set");
            eprintln!("Please set it with: export ANTHROPIC_API_KEY=your-key-here");
            std::process::exit(1);
        }
    };

    println!("=== Dad Jokes Agent ===\n");
    println!("Using Anthropic Claude with current_time capability\n");

    // Set up the Anthropic driver registry
    let mut registry = DriverRegistry::new();
    everruns_anthropic::register_driver(&mut registry);

    // Configure the model
    let model = ModelWithProvider {
        model: "claude-sonnet-4-20250514".to_string(),
        provider_type: LlmProviderType::Anthropic,
        api_key: Some(api_key),
        base_url: None,
    };

    // Build the agent with current_time capability
    let runner = InMemoryAgenticLoop::builder()
        .agent_name("Dad Jokes Bot")
        .system_prompt(DAD_JOKES_SYSTEM_PROMPT)
        .with_driver_registry(model, registry)
        .capability(CurrentTimeCapability)
        .max_iterations(5)
        .build()
        .await?;

    println!("Agent created! Let's hear some jokes...\n");
    println!("---");

    // Conversation 1: Simple greeting
    println!("\nUser: Hello!");
    let result = runner.run_turn("Hello!").await?;
    println!("Dad Bot: {}", result.response);
    if result.tool_calls_count > 0 {
        println!("(Used {} tool call(s))", result.tool_calls_count);
    }

    // Conversation 2: Ask about time (should use get_current_time tool)
    println!("\n---");
    println!("\nUser: What time is it?");
    let result = runner.run_turn("What time is it?").await?;
    println!("Dad Bot: {}", result.response);
    if result.tool_calls_count > 0 {
        println!("(Used {} tool call(s))", result.tool_calls_count);
    }

    // Conversation 3: Topic-based joke
    println!("\n---");
    println!("\nUser: Tell me a joke about programming");
    let result = runner.run_turn("Tell me a joke about programming").await?;
    println!("Dad Bot: {}", result.response);

    // Conversation 4: Classic setup
    println!("\n---");
    println!("\nUser: I'm tired");
    let result = runner.run_turn("I'm tired").await?;
    println!("Dad Bot: {}", result.response);

    // Show conversation summary
    println!("\n---");
    println!("\n=== Conversation Summary ===");
    let messages = runner.messages().await?;
    println!("Total messages: {}", messages.len());
    println!("Total events: {}", runner.event_count().await);

    println!("\n=== Done! ===");
    Ok(())
}
