# Coding Review Agent

A self-contained Framework code-review agent. It attaches a typed trusted-
workspace inspection tool, then runs review instructions through `Engine`.

## Run

![Coding Review Agent terminal demo](demo.gif)

```bash
cargo run -p everruns-coding-review-agent
cargo test -p everruns-coding-review-agent
```

The scripted simulator calls the workspace-inspection tool before the final
review, so CI exercises the agent tool loop. For production, connect the
trusted workspace and configure an Anthropic provider with `claude-sonnet-5` or
the newest compatible Sonnet profile.
