# everruns

The application-facing entrypoint to the [Everruns](https://everruns.com) agentic framework.

`everruns` is a thin, publishable facade over the in-process runtime. Add a single dependency and
run an agent turn — without wiring up `everruns-core` or `everruns-runtime` directly.

The high-level API uses an open, credential-free `ModelSpec` plus runtime
`Provider` contract. Default features stay offline. Custom providers and wire
protocols can be supplied through `everruns` alone; applications do not need to
import `everruns-core` or `everruns-runtime`.

## Example

```rust
use everruns::{Agent, Model};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .instructions("You are a helpful assistant.")
        .model(Model::simulated("4"))
        .build()
        .expect("valid agent");
    let result = agent.session().run("What is 2 + 2?").await?;

    assert_eq!(result.response, "4");
    Ok(())
}
```

## License

MIT
