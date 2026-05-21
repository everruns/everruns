# everruns-a2ui

A2UI component catalog and prompt generation for Everruns generative UI.

This crate is part of the [Everruns](https://everruns.com) ecosystem. It gives
LLMs a compact catalog of A2UI JSON components and rules for emitting `a2ui`
fenced blocks that Everruns clients can render.

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

## License

MIT. See the repository-level `LICENSE` file.
