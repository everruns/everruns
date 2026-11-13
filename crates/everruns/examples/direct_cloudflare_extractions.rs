//! Native Cloudflare extraction using synthetic data (may incur provider charges).
//! CLOUDFLARE_ACCOUNT_ID=... CLOUDFLARE_API_TOKEN=... cargo run -p everruns --features cloudflare --example direct_cloudflare_extractions
use everruns::{ExtractionField, Extractions};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let account = std::env::var("CLOUDFLARE_ACCOUNT_ID")?;
    let token = std::env::var("CLOUDFLARE_API_TOKEN")?;
    let provider = everruns_drivers::cloudflare::provider("cloudflare", &account, token, None);
    let result = Extractions::new("@cf/cloudflare/clairic", provider)
        .about("Invoice INV-42 from Example Studio. Total USD 120.00.")
        .field("invoice_number", ExtractionField::text())
        .field(
            "total",
            ExtractionField::number().instructions("Total amount in USD"),
        )
        .send()
        .await?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
