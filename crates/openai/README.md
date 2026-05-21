# everruns-openai

OpenAI LLM provider implementation for Everruns.

This crate is part of the [Everruns](https://everruns.com) ecosystem. It
registers OpenAI drivers with `everruns-core`, including the recommended
Responses API driver and a Chat Completions compatibility driver.

## Quick Example: Agent With OpenAI

```rust,no_run
use everruns_core::{
    CapabilityRegistry, DriverRegistry, LlmProviderType, ModelWithProvider, PlatformDefinition,
};
use everruns_runtime::InProcessRuntimeBuilder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut drivers = DriverRegistry::new();
    everruns_openai::register_driver(&mut drivers);

    let platform = PlatformDefinition::new(CapabilityRegistry::new(), drivers);
    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(platform)
        .default_model(ModelWithProvider {
            model: "gpt-5.4-mini".into(),
            provider_type: LlmProviderType::Openai,
            api_key: Some(std::env::var("OPENAI_API_KEY")?),
            base_url: None,
        })
        .single_session(|s| {
            s.harness("assistant", "You are a helpful assistant.")
                .agent("openai-agent", "Answer clearly and concisely.")
                .session_title("OpenAI example")
        })
        .build()
        .await?;

    let session_id = runtime.default_session_id().expect("single_session id");
    let result = runtime
        .run_text_turn(session_id, "Write one sentence about reliable agents.")
        .await?;

    println!("{}", result.response);
    Ok(())
}
```

## Driver-Only Example

```rust
use everruns_openai::OpenAILlmDriver;

let driver = OpenAILlmDriver::new("your-api-key");
assert!(!driver.uses_custom_url());
```

## License

MIT. See the repository-level `LICENSE` file.
