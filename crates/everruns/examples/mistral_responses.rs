//! Mistral Responses through the typed connection configuration.
//!
//! Set MISTRAL_API_KEY, then run:
//! `cargo run -p everruns --features mistral --example mistral_responses -- "Say hello"`
//! Running this example makes a real, billable provider request.

use everruns::{Mistral, MistralApi, Model};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let prompt = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let prompt = if prompt.is_empty() {
        "Say hello."
    } else {
        &prompt
    };
    let provider = Mistral::from_env()?.api(MistralApi::Responses);
    let model = Model::new("mistral-large-4", provider);
    println!("{}", model.complete(prompt).await?);
    Ok(())
}
