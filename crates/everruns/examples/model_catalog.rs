//! Ask a provider which models it offers, then run the one that was picked.
//!
//! Requires OPENAI_API_KEY:
//! ```text
//! cargo run -p everruns --features openai --example model_catalog
//! ```

use everruns::{OpenAI, Provider, models};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("OpenAI's own catalog, over HTTP.\n");
    let provider: Provider = OpenAI::from_env()?.into();

    // One provider call. Ids come back exactly as chat calls expect them,
    // merged with the profile registry for names, descriptions, and limits the
    // provider's API does not report.
    let catalog = match models::list(provider).await {
        Ok(catalog) => catalog,
        // Not every provider can enumerate its models; that is not a failure.
        Err(models::CatalogError::NoCatalog) => {
            println!("This provider offers no catalog; keep your curated list.");
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };

    for model in catalog.iter().take(10) {
        let name = model.display_name().unwrap_or(model.id());
        print!("{:<28} {name}", model.id());
        if let Some(window) = model.context_window() {
            print!("  ({window} tokens)");
        }
        if model.supports_reasoning() {
            print!("  [reasoning]");
        }
        println!();
    }

    // A selection converts straight back into a model, bundled with the
    // provider it was discovered through: no string handling, nothing to
    // reconfigure.
    let Some(picked) = catalog.into_iter().find(|model| model.supports_tools()) else {
        println!("\nNo tool-calling model in this catalog.");
        return Ok(());
    };
    println!("\npicked {}", picked.id());
    if let Some(profile) = picked.profile()
        && let Some(cost) = profile.cost.as_ref()
    {
        println!(
            "input ${:.2} / output ${:.2} per million tokens",
            cost.input, cost.output
        );
    }

    let answer = picked
        .model()
        .complete("Name the three primary colors.")
        .await?;
    println!("\n{answer}");

    Ok(())
}
