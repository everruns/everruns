//! Opt-in synthetic-document live qualification executable. Never runs in default CI.
//! REVIEW provider pricing, then set EVERRUNS_EXTRACTIONS_LIVE=1 and credentials.
#![cfg(all(
    not(target_arch = "wasm32"),
    feature = "openrouter",
    feature = "cloudflare"
))]
use everruns_contracts::extractions::{ExtractionField, ExtractionRequest};
fn request(model: &str) -> ExtractionRequest {
    let mut r =
        ExtractionRequest::new("Invoice INV-42 issued by Example Studio. Total USD 120.00.").field(
            "invoice_number",
            ExtractionField::text().instructions("Invoice identifier"),
        );
    r.model = Some(model.into());
    r
}
async fn openrouter_extracts_synthetic_invoice() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(
        std::env::var("EVERRUNS_EXTRACTIONS_LIVE").as_deref(),
        Ok("1")
    );
    let provider = everruns_drivers::openrouter::provider(
        "live-openrouter",
        std::env::var("OPENROUTER_API_KEY")?,
    );
    let response = provider.extract(request("cloudflare/clairic")).await?;
    assert_eq!(response.answers["invoice_number"].value, "INV-42");
    assert!(!response.truncated);
    Ok(())
}
async fn cloudflare_extracts_synthetic_invoice() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(
        std::env::var("EVERRUNS_EXTRACTIONS_LIVE").as_deref(),
        Ok("1")
    );
    let provider = everruns_drivers::cloudflare::provider(
        "live-cloudflare",
        &std::env::var("CLOUDFLARE_ACCOUNT_ID")?,
        std::env::var("CLOUDFLARE_API_TOKEN")?,
        None,
    );
    let response = provider.extract(request("@cf/cloudflare/clairic")).await?;
    assert_eq!(response.answers["invoice_number"].value, "INV-42");
    assert!(!response.truncated);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match std::env::var("EXTRACTIONS_LIVE_PROVIDER")?.as_str() {
        "openrouter" => openrouter_extracts_synthetic_invoice().await,
        "cloudflare" => cloudflare_extracts_synthetic_invoice().await,
        _ => Err("Select EXTRACTIONS_LIVE_PROVIDER=openrouter or cloudflare".into()),
    }
}
