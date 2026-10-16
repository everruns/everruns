---
title: Everruns agents gain a Durable Object authority
description: Run ordinary Rust agents with object-owned input, logs, leases and alarms, using the same recovery driver as native Everruns workers.
date: 2026-10-16
kind: feature
significance: minor
products: [framework]
surfaces: [code]
links:
  docs: [/framework/durable-objects/]
---

![Everruns plus Cloudflare Durable Objects](./integration@2x.png)

An experimental, source-only adapter now runs ordinary Rust agents with a Durable
Object owning durable admission, session events, execution leases and alarms.
The executor shares the native worker's recovery driver, so tools and agent logic
stay in Rust.

```sh
cargo run -p everruns-durable-object --example native
```

Configure the Worker and service secrets first, as described in
[Durable Object authority](/framework/durable-objects/). Framework agents and
registered serve agents can supply the execution host. A workerd test kills the
native executor after a tool effect and verifies alarm-driven recovery without
resending input. Production Cloudflare deployment, multi-node celld and Wasm
execution remain unqualified. This profile does not change the guarantees of
ordinary Framework sends or the existing celld snapshot example.
