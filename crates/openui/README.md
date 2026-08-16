# everruns-openui

> OpenUI component library and prompt generator for Everruns generative UI.

[![Crates.io](https://img.shields.io/crates/v/everruns-openui.svg)](https://crates.io/crates/everruns-openui)
[![Documentation](https://docs.rs/everruns-openui/badge.svg)](https://docs.rs/everruns-openui)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-openui` builds the system-prompt text that teaches an LLM to produce
`openui` fenced blocks, generative UI that Everruns clients that understand
OpenUI Lang can render directly in a conversation. It ships a static component library
and lets you extend the prompt with your own rules and examples.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. See
[`everruns-a2ui`](https://crates.io/crates/everruns-a2ui) for the JSON-component
variant of generative UI.

## Quick Example

```rust
use everruns_openui::{PromptOptions, default_library, generate_prompt};

let prompt = generate_prompt(
    default_library(),
    &PromptOptions {
        additional_rules: vec!["Prefer compact tables for operational data.".into()],
        ..PromptOptions::default()
    },
);

assert!(prompt.contains("```openui"));
```

## What It Provides

- A static OpenUI component library
- Prompt generation for OpenUI Lang output
- Custom prompt options for extra rules and examples

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-openui)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
