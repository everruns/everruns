# everruns-a2ui

> A2UI component catalog and prompt generator for Everruns generative UI.

[![Crates.io](https://img.shields.io/crates/v/everruns-a2ui.svg)](https://crates.io/crates/everruns-a2ui)
[![Documentation](https://docs.rs/everruns-a2ui/badge.svg)](https://docs.rs/everruns-a2ui)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-a2ui` gives an LLM a compact catalog of A2UI JSON components plus the
rules for emitting `a2ui` fenced blocks, JSON component trees that Everruns
clients can render as generative UI. It ships a default catalog with component
and prop metadata, and lets you extend the prompt with your own rules.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. See
[`everruns-openui`](https://crates.io/crates/everruns-openui) for the OpenUI Lang
variant of generative UI.

## Quick Example

```rust
use everruns_a2ui::{PromptOptions, default_catalog, generate_prompt};

let prompt = generate_prompt(
    default_catalog(),
    &PromptOptions {
        additional_rules: vec!["Use action buttons only for clear next steps.".into()],
        ..PromptOptions::default()
    },
);

assert!(prompt.contains("```a2ui"));
```

## What It Provides

- A default A2UI catalog
- Prompt generation for JSON component trees
- Component and prop metadata for renderer-compatible output

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-a2ui)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
