# Coding Review Agent

A self-contained Framework code-review agent. It uses Anthropic's
`claude-sonnet-5`, inspects the included `sample_payment.rs`, and produces a
real code review.

## Run

![Coding Review Agent terminal demo](demo.gif)

```bash
ANTHROPIC_API_KEY=... cargo run -p everruns-coding-review-agent
cargo test -p everruns-coding-review-agent
```

Pass a different review request after `--`. The test only validates agent
construction; `cargo run` makes the real provider call.
