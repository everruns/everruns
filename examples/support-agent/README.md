# Support Agent

A self-contained Everruns Framework customer-support agent. It builds an
`Agent`, exposes a typed customer-lookup tool, and runs a session through
`Engine` without credentials or network access.

## Run

![Support Agent terminal demo](demo.gif)

```bash
cargo run -p everruns-support-agent
cargo test -p everruns-support-agent
```

The example uses a scripted `Model::simulated_with_config(...)`: it calls the
customer lookup tool before returning its answer, so CI exercises a real agent
tool loop. For production, configure an OpenAI provider in the embedding application
and replace the simulated model with the recommended `gpt-5.6-terra` profile.
