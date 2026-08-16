//! A reusable typed capability with metadata, structured errors, execution
//! context, progress, and a non-string result.
//!
//! ```text
//! OPENAI_API_KEY=sk-... cargo run -p everruns --features openai --example advanced_capability
//! ```

use everruns::{Agent, Engine, OpenAI, SessionEventKind, capability};

#[derive(capability::Deserialize, capability::JsonSchema)]
#[serde(crate = "everruns::capability::serde")]
#[schemars(crate = "everruns::capability::schemars")]
struct LookupInput {
    sku: String,
}

#[derive(capability::Serialize, capability::JsonSchema)]
#[serde(crate = "everruns::capability::serde")]
#[schemars(crate = "everruns::capability::schemars")]
struct Product {
    sku: String,
    name: String,
    price_cents: u64,
    tags: Vec<String>,
}

struct ProductLookup;

#[capability::async_trait]
impl capability::Handler for ProductLookup {
    type Input = LookupInput;
    type Output = Product;
    type Error = capability::Error;

    fn name(&self) -> &str {
        "lookup_product"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Look up product")
    }

    fn description(&self) -> &str {
        "Look up one product by its exact SKU."
    }

    fn hints(&self) -> capability::Hints {
        capability::Hints::default()
            .readonly(true)
            .idempotent(true)
            .metadata(capability::serde_json::json!({ "domain": "catalog" }))
    }

    async fn execute(
        &self,
        input: Self::Input,
        context: capability::Context,
    ) -> Result<Self::Output, Self::Error> {
        context.progress("Searching the product catalog").await;
        if input.sku != "EVR-42" {
            return Err(
                capability::Error::user("product_not_found", "No product has that SKU")
                    .details(capability::serde_json::json!({ "sku": input.sku })),
            );
        }
        Ok(Product {
            sku: input.sku,
            name: "Everruns Field Guide".into(),
            price_cents: 4200,
            tags: vec!["agents".into(), "rust".into()],
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let catalog = capability::Definition::new(
        "product_catalog",
        "Product catalog",
        "Application-owned product data.",
    )
    .instructions("Use exact SKUs. Never invent catalog entries.")
    .metadata(capability::serde_json::json!({ "owner": "commerce" }))
    .tool(ProductLookup);

    let agent = Agent::builder()
        .name("catalog-assistant")
        .instructions("Answer questions using the product catalog capability.")
        .provider(OpenAI::from_env()?)
        .model("gpt-5.6-terra")
        .capability(catalog)
        .build()?;

    let session = Engine::new().create(agent.clone());
    let mut events = session.events();
    let pending = session.send("What is product EVR-42?").await?;
    while let Some(event) = events.recv().await? {
        if let SessionEventKind::ToolProgress { message, .. } = &event.kind {
            eprintln!("progress: {message}");
        }
        if event.turn_id.as_deref() == Some(&pending.turn_id) && event.kind.is_terminal() {
            break;
        }
    }

    let turn = pending.wait().await?;
    println!("{}", turn.response);
    Ok(())
}
