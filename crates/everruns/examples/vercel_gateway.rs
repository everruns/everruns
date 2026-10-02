//! Reach many upstream vendors through one account with Vercel AI Gateway.
//!
//! The gateway speaks the Open Responses specification, which Everruns already
//! implements, so the driver adds only identity, auth, and model discovery. For
//! the caller that means a gateway provider is configured exactly like a direct
//! one — the only visible difference is the model id, which is namespaced
//! `vendor/model` and passed through to the upstream vendor unchanged.
//!
//! Set `AI_GATEWAY_API_KEY` to an AI Gateway API key (a Vercel OIDC token works
//! in the same field), then:
//!
//! ```text
//! cargo run -p everruns --example vercel_gateway
//! ```
//!
//! An optional positional argument replaces the model id, so one account can be
//! pointed at a different vendor without touching the code:
//!
//! ```text
//! cargo run -p everruns --example vercel_gateway -- openai/gpt-6-astra
//! ```

use everruns::{Agent, Engine};
use everruns_drivers::vercel;

/// Return the population of a city, from a tiny fixed table.
///
/// A tool keeps this example honest: it shows the gateway carrying a real tool
/// call and its result, not just plain text.
#[everruns::tool]
async fn city_population(city: String) -> Result<String, String> {
    let population = match city.trim().to_lowercase().as_str() {
        "lisbon" => "545,000",
        "kyiv" => "2,950,000",
        "osaka" => "2,750,000",
        other => return Err(format!("no population on file for {other}")),
    };
    Ok(format!("{city}: {population}"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    // Model ids are `vendor/model`. The gateway routes on the vendor prefix, so
    // switching vendors is a string change rather than a provider change.
    let model = args.next().unwrap_or_else(|| "zai/glm-4.6".into());
    if args.next().is_some() {
        return Err("Usage: vercel_gateway [VENDOR/MODEL]".into());
    }

    // `from_env` reads the variables the driver itself declares — here
    // `AI_GATEWAY_API_KEY`. It is the standalone/CLI/dev path: a server resolves
    // credentials from storage and never reads the environment. The error names
    // the missing variable rather than deferring to a 401 on the first request.
    let provider = vercel::from_env("vercel")?;

    let agent = Agent::builder()
        .name("vercel-gateway")
        .instructions("Use city_population for population questions. Be terse.")
        .provider(provider)
        .model(model.clone())
        .tool(city_population())
        .build()?;

    let session = Engine::new().create(agent);
    println!("model: {model}\n");

    let turn = session
        .send("How many people live in Lisbon, and in Kyiv?")
        .await?
        .wait()
        .await?;

    println!("response: {}", turn.response);
    println!(
        "iterations: {}, tool calls: {}",
        turn.iterations, turn.tool_calls
    );

    Ok(())
}
