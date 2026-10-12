---
title: "TLS backend and timezone data become explicit features"
description: "Bashkit 0.17 builds without default features must pick a rustls crypto backend for http_client and enable tzdata to use named time zones."
date: 2026-08-21
kind: breaking
significance: notable
products: [bashkit]
surfaces: [code]
release: "0.17.0"
links:
  docs: ["https://github.com/everruns/bashkit/blob/main/docs/networking.md"]
  prs: [everruns/bashkit#2317, everruns/bashkit#2321]
---

Bashkit 0.17.0 changes two Cargo features. Default builds are unchanged; only
builds with `default-features = false` need to act.

- `http_client` no longer implies a rustls crypto backend. Add `ring` (the
  previous behavior, on by default) or `aws-lc-rs` for a FIPS-capable backend.
  Enabling neither is a compile error.
- Timezone data is behind the `tzdata` feature. Without it, `date` with
  `TZ=<named zone>` exits 1 instead of printing a UTC time.

```toml
bashkit = { version = "0.17", default-features = false, features = ["http_client", "ring", "tzdata"] }
```

Non-ASCII domain names in HTTP allowlists are now rejected rather than
normalized; use their punycode form.

See [Networking](https://github.com/everruns/bashkit/blob/main/docs/networking.md), [#2317](https://github.com/everruns/bashkit/pull/2317) and [#2321](https://github.com/everruns/bashkit/pull/2321).
