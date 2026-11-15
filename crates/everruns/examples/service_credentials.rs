//! Application-owned Cursor, Browserless and E2B credentials; real native tools.
//! Choose one: service_credentials cursor|browserless|e2b [TASK]
//! cargo run -p everruns --features openai,cursor,browserless,e2b --example service_credentials -- e2b
use everruns::{Agent, Engine, Model, OpenAI, services::ApiKey};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service = std::env::args()
        .nth(1)
        .ok_or("Choose cursor, browserless, or e2b")?;
    let builder=Agent::builder().instructions("Use the selected native integration to complete the request. Never print credentials. Release browsers and delete sandboxes you create after finishing, including after a tool failure.").model(Model::new("gpt-6.1-sol",OpenAI::from_env()?));
    let (builder, default_task) = match service.as_str() {
        "cursor" => (
            builder.cursor(ApiKey::new(std::env::var("CURSOR_API_KEY")?)),
            "List available Cursor models. Do not launch an agent.",
        ),
        "browserless" => (
            builder.browserless(ApiKey::new(std::env::var("BROWSERLESS_API_TOKEN")?)),
            "Open https://example.com, report the title, and close the browser.",
        ),
        "e2b" => (
            builder.e2b(ApiKey::new(std::env::var("E2B_API_KEY")?)),
            "Create a sandbox, run python3 to compute sum(range(100)), and delete the sandbox.",
        ),
        _ => return Err("Choose cursor, browserless, or e2b".into()),
    };
    let agent = builder.build()?;
    let session = Engine::new().create(agent);
    let task = std::env::args()
        .nth(2)
        .unwrap_or_else(|| default_task.into());
    let turn = session.run(task).await?;
    if !turn.success {
        return Err(format!("Agent failed: {:?}", turn.error).into());
    }
    println!("{}", turn.response);
    Ok(())
}
