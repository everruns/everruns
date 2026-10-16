# Provider drivers inside workerd

A test-only Worker runs the shared Chat Completions and Responses protocols,
OpenAI's two chat drivers, and OpenRouter through the example's host `fetch`
adapter, both directly and inside a Durable Object. A local scripted HTTP
server checks tool-call replay, exact headers and provider options, JSON
responses, empty response bodies, retry handling, structured errors and redirect refusal. It splits
SSE at byte boundaries and withholds completion until WASM acknowledges a text
delta, so a buffered adapter cannot pass. No provider credential is needed.

Install Rust's `wasm32-unknown-unknown` target, Node 24 and
`cargo install worker-build --locked --version 0.8.7`, then run from the repo root:

```sh
bash scripts/lib/test-wasm-providers.sh
```

This fixture is deliberately separate from real-model examples. Its only
outbound endpoints and credentials are created locally by the test runner.
