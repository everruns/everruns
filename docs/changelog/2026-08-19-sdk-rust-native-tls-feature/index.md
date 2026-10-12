---
title: "Everruns Rust SDK can use native TLS for smaller binaries"
description: "TLS in the Everruns Rust SDK is now a Cargo feature: keep the default rustls-tls or pick native-tls, which cuts a minimal client from 6.5 MiB to 3.0 MiB."
date: 2026-08-19
kind: improvement
significance: minor
products: [sdk]
surfaces: [code]
release: "0.2.1"
links:
  docs: ["/features/sdk/"]
  prs: [everruns/sdk#110, everruns/sdk#109]
---

TLS in the `everruns-sdk` crate is now a Cargo feature. The default
`rustls-tls` bundles rustls and needs no system TLS library. `native-tls` links
the platform TLS stack instead, which needs OpenSSL or the platform equivalent
on the build and target machines.

```toml
everruns-sdk = { version = "0.2", default-features = false, features = ["native-tls"] }
```

On a minimal client that creates a session and streams events, a stripped
release build is 6.5 MiB with `rustls-tls` and 3.0 MiB with `native-tls`.
Leaner tokio, futures and reqwest feature sets also trim the default build, and
the SDK moves to reqwest 0.13.

See [SDKs](/features/sdk/), [#110](https://github.com/everruns/sdk/pull/110) and [#109](https://github.com/everruns/sdk/pull/109).
