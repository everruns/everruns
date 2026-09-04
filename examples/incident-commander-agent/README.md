# Incident Commander Agent

A self-contained Framework incident-coordination agent. It uses Meta Model
API's `muse-spark-1.3`, records a bounded incident update, and proposes safe
next actions from a real model response.

## Run

![Incident Commander Agent terminal demo](demo.gif)

```bash
MODEL_API_KEY=... cargo run -p everruns-incident-commander-agent
cargo test -p everruns-incident-commander-agent
```

Pass a different incident scenario after `--`. The test only validates agent
construction; `cargo run` makes the real provider call.

`META_API_KEY` is also accepted for environments that use that name.
