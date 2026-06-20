# everruns-fireworks

Fireworks AI provider driver for [Everruns](https://everruns.com).

[Fireworks AI](https://fireworks.ai) serves open models (Llama, Qwen, DeepSeek,
GLM, Kimi, gpt-oss, ...) behind an OpenAI-compatible Chat Completions API. This
crate implements the `ChatDriver` contract from `everruns-core` by wrapping the
core `OpenAIProtocolChatDriver` and tagging it with `DriverId::Fireworks`.

## Capabilities

- **Chat completions** — streaming, tool calling, vision, and structured output,
  via the OpenAI-compatible Chat Completions API.
- **Model discovery** — Fireworks' `/models` endpoint advertises rich metadata
  (`supports_chat`, `supports_tools`, `supports_image_input`, `context_length`),
  which this crate parses into capability profiles at sync time.

## Authentication

Fireworks authenticates with a single API key, sent as a bearer token. Create a
key at <https://fireworks.ai> under **Account → API Keys**.

## Usage

```rust
use everruns_core::DriverRegistry;
use everruns_fireworks::register_driver;

let mut registry = DriverRegistry::new();
register_driver(&mut registry);
assert!(registry.has_driver(&everruns_core::DriverId::Fireworks));
```
