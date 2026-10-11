---
title: "Runs as a WebAssembly component without JS or WASI"
description: "Bashkit can run as a component-model WebAssembly guest in plain wasmtime or a Hyperlight micro-VM, with no JavaScript engine and no WASI needed."
date: 2026-08-21
kind: feature
significance: notable
products: [bashkit]
surfaces: [code]
release: "0.17.0"
links:
  docs: ["https://github.com/everruns/bashkit/tree/main/examples/hyperlight"]
  prs: [everruns/bashkit#2311, everruns/bashkit#2310]
---

Bashkit can now run as a WebAssembly component that exposes shell execution
over WIT, in a host with no JavaScript engine and no WASI implementation. That
covers plain wasmtime and the `no_std` wasmtime that hyperlight-wasm compiles
into a Hyperlight micro-VM guest. `examples/hyperlight/` builds such a guest
and runs a script through it.

Dependencies that assumed a JavaScript host, such as the clock, timers and
entropy source, moved behind a new `wasm_js` feature that the JS packages
enable and other builds leave off. The `bashkit` crate also builds for
`wasm32-wasip2`, checked in CI.

See the [Hyperlight example](https://github.com/everruns/bashkit/tree/main/examples/hyperlight), [#2311](https://github.com/everruns/bashkit/pull/2311) and [#2310](https://github.com/everruns/bashkit/pull/2310).
