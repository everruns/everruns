---
title: Framework direct model calls with Model::complete
description: The Everruns Framework adds a promoted path for one prompt and one answer, and each driver now reads its vendor's standard credential environment variables.
date: 2026-09-16
kind: feature
significance: notable
products: [framework]
surfaces: [code]
release: "0.29.0"
links:
  docs: [/framework/direct-model-calls/, /framework/models-and-providers/]
  prs: [everruns/everruns#3642, everruns/everruns#3668]
---

Work that is one prompt and one answer no longer needs an agent.
`Model::complete` covers the one-line case, and `Model::completion` adds system
messages, replayed context, per-call controls, streaming and a typed
`CompletionError`.

```rust
let model = Model::new("gpt-6.1-sol", OpenAI::from_env()?);
let answer = model.complete("Name the three primary colors.").await?;
```

Each driver now declares its own credential variables, following the vendor's
SDK, for example `GEMINI_API_KEY` or `GOOGLE_API_KEY`, the AWS variables for
Bedrock, and `LLAMA_API_KEY` for Meta. Provider model catalogs are also exposed on
the facade crate.

See [Direct model calls](/framework/direct-model-calls/) and
[Models and providers](/framework/models-and-providers/). [#3642](https://github.com/everruns/everruns/pull/3642), [#3668](https://github.com/everruns/everruns/pull/3668)
