//! Native field extraction, no agent or chat conversion.
//! OPENROUTER_API_KEY=... cargo run -p everruns --features utility-llm --example direct_extractions
//! Uses synthetic invoice text only. This calls a paid provider; review its pricing first.
use everruns::{ExtractionField, Extractions};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("OPENROUTER_API_KEY")?;
    let model = std::env::var("EXTRACTION_MODEL").unwrap_or_else(|_| "cloudflare/clairic".into());
    let extraction = Extractions::new(
        model,
        everruns_drivers::openrouter::provider("openrouter", key),
    );
    // Optional output of a separate local converter such as Markkit. Review it
    // before opting in: this file is sent to the configured provider.
    let state = match std::env::var("EXTRACTION_INPUT_FILE") {
        Ok(path) => std::fs::read_to_string(path)?,
        Err(_) => "Invoice INV-42 from Example Studio. Total USD 120.00.".into(),
    };
    let outcome = extraction
        .about(state)
        .field("invoice_number", ExtractionField::text())
        .field(
            "total",
            ExtractionField::number().instructions("Invoice total in USD"),
        )
        .send()
        .await?;
    println!("{}", serde_json::to_string_pretty(&outcome)?);
    Ok(())
}
