---
title: "Jev 1.14 is available for typed decisions"
description: "Select Jev 1.14 through TypeSafe or OpenRouter for calibrated text decisions, with matching provider IDs and existing Jev 1.13 models preserved."
date: 2026-10-31
kind: improvement
significance: minor
products: [platform, cloud, framework]
surfaces: [ui, api, code]
links:
  docs: [/integrations/typesafe/]
---

**In the UI:** Sync a TypeSafe or OpenRouter provider, then select the **Jev 1.14**
decision profile. Direct TypeSafe uses `jev-1.14.0`; OpenRouter uses
`typesafe/jev-1.14`. Jev 1.13 remains available, and the latest alias now uses
OpenRouter's required `~typesafe/jev-latest` spelling.

**In code:** Pin the release when constructing a decision client:

```rust
let decisions = Decisions::new("jev-1.14.0", TypeSafeAI::from_env()?);
```

The [TypeSafe integration](/integrations/typesafe/) supports text Noul, Choice,
and Score questions with the existing default limits. Experimental image, Box,
and direct long-context features are not enabled.
