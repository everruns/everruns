# Builder and verifier

The builder runs on `openai/gpt-6.2-sol`. Before it answers, it spawns a
"Verifier" subagent with `model: "second_opinion"`, a helper model whose target
is the Default job of a connected provider other than the builder's. The
verifier reviews the function and the builder fixes what it finds.

`max_subagent_depth = 1` keeps the verifier from spawning a verifier of its own:
a subagent gets the same instructions and capabilities as its parent.

Needs OpenAI and one more provider (Anthropic, for example) connected.

```bash
everruns agents import examples/agents/builder-verifier
everruns sessions create --agent builder-verifier
everruns chat -s <session_id> "Write a function that parses ISO 8601 durations like P1DT2H30M into seconds."
everruns tasks list   # the Verifier task: spec.model = second_opinion, spec.model_pick = agent
```

A run on a local stack with OpenAI and Anthropic connected (2026-11-20): the
Verifier task recorded `model: second_opinion`, `model_pick: agent` and ran on
Claude Sonnet 5.5 (the Anthropic Default job). It traced valid inputs and edge
cases and reported "No issues", and the builder answered:

> The verifier found no issues within the stated scope and noted the usual
> floating-point rounding and overflow limits.
