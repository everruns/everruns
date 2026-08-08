# everruns

The application-facing entrypoint to the [Everruns](https://everruns.com) agentic framework.

`everruns` is a thin, publishable facade over the in-process runtime. Add a single dependency and
run an agent turn — without wiring up `everruns-core` or `everruns-runtime` directly.

This first release is a **compatibility facade**: it moves no engine code and re-exports the
minimum needed to construct and run an in-process session. Default features stay offline — no
provider, MCP, filesystem, SQLx, server, or worker integrations are activated by default. APIs not
yet promoted onto the facade remain reachable through the escape-hatch `everruns::core` and
`everruns::runtime` modules.

## Example

```rust
use everruns::{DriverId, InProcessRuntimeBuilder, InputMessage, LlmSimConfig, ResolvedModel};

#[tokio::main]
async fn main() -> Result<(), everruns::AgentLoopError> {
    let runtime = InProcessRuntimeBuilder::new()
        .single_session(|s| {
            s.harness("assistant", "You are a helpful assistant.")
                .agent("assistant-agent", "Answer the user.")
        })
        .llm_sim(LlmSimConfig::fixed("4"))
        .default_model(ResolvedModel {
            model: "llmsim-model".into(),
            provider_type: DriverId::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
            provider_metadata: None,
        })
        .build()
        .await?;

    let session_id = runtime.default_session_id().expect("single_session id");
    let result = runtime
        .run_turn(session_id, InputMessage::user("What is 2 + 2?"))
        .await?;

    assert_eq!(result.response, "4");
    Ok(())
}
```

## License

MIT
