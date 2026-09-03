# Everruns Support Agent

A self-contained Framework agent for developer support. It combines scoped
troubleshooting instructions with a typed documentation-search tool and runs a
session through `Engine`.

## Run

![Everruns Support Agent terminal demo](demo.gif)

```bash
cargo run -p everruns-framework-support-agent
cargo test -p everruns-framework-support-agent
```

The scripted simulator calls the documentation-search tool before the final
answer, so CI exercises the agent tool loop. For production, configure an
Anthropic provider in the embedding application and select `claude-opus-5` or
the newest compatible Opus profile.
