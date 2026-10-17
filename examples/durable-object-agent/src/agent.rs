//! The application definition. Identical on native, Cloudflare and celld.
use everruns::{Agent, BuildError, FunctionTool, Model, ToolCallContext};
use serde_json::{Value, json};

pub fn counter(model: Model) -> Result<Agent, BuildError> {
    Agent::builder()
        .name("counter")
        .instructions("Use bump_counter when asked to increment. Report its result briefly.")
        .model(model)
        .tool(
            FunctionTool::with_context(
                "bump_counter",
                "Increment the session counter once.",
                json!({"type":"object","properties":{}}),
                |context: ToolCallContext, _: Value| async move {
                    let current = context
                        .get_value("counter")
                        .await?
                        .map(|value| {
                            value
                                .parse::<u64>()
                                .map_err(|_| "invalid counter".to_string())
                        })
                        .transpose()?
                        .unwrap_or(0);
                    let next = current.checked_add(1).ok_or("counter overflow")?;
                    context.set_value("counter", &next.to_string()).await?;
                    Ok::<_, String>(json!({"counter":next}))
                },
            )
            .concurrency_class("session_storage"),
        )
        .build()
}
