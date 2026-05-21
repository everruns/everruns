# everruns-openui

OpenUI component definitions and prompt generation for Everruns generative UI.

This crate is part of the [Everruns](https://everruns.com) ecosystem. It builds
the system-prompt text that teaches an LLM to produce `openui` fenced blocks for
rendering in clients that understand OpenUI Lang.

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

## License

MIT. See the repository-level `LICENSE` file.
