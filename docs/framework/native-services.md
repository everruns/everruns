---
title: Native services
description: Bind application-owned credentials to Cursor, Browserless and E2B tools, with optional image generation using the shared native integrations.
---

Enable each integration independently with `cursor`, `browserless`, or `e2b`. These values install the existing native tool implementations on real Engine contexts. They do not require Server connection records or its account-selection UI.

```rust
use everruns::{Agent, Model, services::ApiKey};

let agent = Agent::builder()
    .instructions("Delegate coding tasks, inspect results, and clean up resources when done.")
    .model(Model::new("gpt-6.1-sol", everruns::OpenAI::from_env()?))
    .cursor(ApiKey::new(std::env::var("CURSOR_API_KEY")?))
    .browserless(ApiKey::new(std::env::var("BROWSERLESS_API_TOKEN")?))
    .e2b(ApiKey::new(std::env::var("E2B_API_KEY")?))
    .build()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`ApiKey` redacts `Debug`; credentials are never serialized with agent configuration. Implement `services::Credentials` for vault-backed or rotating keys. Credential acquisition happens when a vendor tool executes; errors are sanitized, and an explicitly bound provider does not fall back to another application's account. These credentials grant access to the selected vendor account, so use scoped vendor keys and agent instructions appropriate to that account.

| Service | Native behavior |
| --- | --- |
| Cursor | Launch, inspect, follow up, list and delete Cloud Agents; inspect conversation/model/repository/key metadata |
| Browserless | Open/reconnect/close browser sessions, navigate, read, interact and capture results using the shared browser operations |
| E2B | Create/list sandboxes, execute commands, read/write files, and manage pause/resume/delete lifecycle |

Browser and sandbox state belongs to the Engine session's service storage. Use a durable Engine profile and protected secret storage when resources must survive process loss. E2B execution uses the provider's isolation and timeout policies. Explicitly close browsers and delete sandboxes when finished; application operators remain responsible for account quotas and orphan cleanup after a host crash. The Framework does not run the Server's fleet cleanup scheduler.

The [native services example](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/service_credentials.rs) uses a real model and one selected service:

```bash
cargo run -p everruns --features openai,cursor,browserless,e2b \
  --example service_credentials -- e2b
```

Replace `e2b` with `cursor` or `browserless`. The default Cursor task only lists models. The browser task opens a public page and closes the browser; the E2B task creates a sandbox, runs Python, then deletes it. Custom tasks can incur vendor costs.

Image generation is available independently through `images` and `everruns::images`. See the [image generation example](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/image_generation.rs) for real provider credentials and generated artifacts. Named agent delegation uses the existing [coordinator API](/framework/coordination/); members retain their own model, instructions and capabilities.
