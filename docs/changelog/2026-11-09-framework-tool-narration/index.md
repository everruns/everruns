---
title: Narrate your own Framework tools
description: Framework tools and capability handlers can now set the human-readable line shown for each call, or keep the generic localized narration.
date: 2026-11-09
kind: improvement
significance: minor
products: [framework]
surfaces: [code]
links:
  docs: [/framework/advanced-capabilities/]
---

Tool events carry a short `narration` line that clients show for each call.
Application tools always got the generic line, such as `Running Lookup Record`.
`capability::Handler`, `FunctionTool` and `#[everruns::tool]` now take an
optional narrator. Return `None` to keep the generic line. With the default
`macros` feature:

```rust
use everruns::narration;

/// Convert Celsius to Fahrenheit.
#[everruns::tool(narrate = |args: &FahrenheitArgs, _call: &everruns::ToolNarration<'_>| {
    Some(format!("Converting {} °C", narration::truncate(&args.celsius.to_string(), 16)))
})]
async fn fahrenheit(celsius: f64) -> f64 {
    celsius * 1.8 + 32.0
}
```

The `everruns::narration` helpers bound and redact argument values and localize
verbs the way built-in tools do. See
[Tool narration](/framework/advanced-capabilities/#tool-narration).
