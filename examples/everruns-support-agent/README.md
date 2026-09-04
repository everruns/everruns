# Everruns Support Agent

A self-contained Framework agent for developer support. It uses Anthropic's
`claude-opus-5`, consults authoritative Everruns documentation links, and
answers a real troubleshooting question.

## Run

![Everruns Support Agent terminal demo](demo.gif)

```bash
ANTHROPIC_API_KEY=... cargo run -p everruns-framework-support-agent
cargo test -p everruns-framework-support-agent
```

Pass a different support question after `--`. The test only validates agent
construction; `cargo run` makes the real provider call.
