---
title: "Everruns SDKs default to the Everruns Cloud API URL"
description: "An Everruns SDK client created with only EVERRUNS_API_KEY now reaches Everruns Cloud instead of a placeholder URL; EVERRUNS_API_URL still overrides it."
date: 2026-10-02
kind: fix
significance: minor
products: [sdk]
surfaces: [code]
release: "0.3.0"
links:
  docs: ["/features/sdk/"]
  prs: [everruns/sdk#114]
---

The Python, TypeScript and Rust SDKs defaulted to the placeholder
`https://custom.example.com/api`, so a client created with only
`EVERRUNS_API_KEY` could not reach anything. The default is now
`https://app.everruns.com/api`.

Self-hosted deployments keep pointing the client at their server with
`EVERRUNS_API_URL`:

```bash
export EVERRUNS_API_URL=https://everruns.example.internal/api
```

See [SDKs](/features/sdk/) and [#114](https://github.com/everruns/sdk/pull/114).
