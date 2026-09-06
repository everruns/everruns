---
title: Support Agent
description: Answer customer support questions with GPT-5.6 Terra and a typed customer-state tool.
---

The [Support Agent source](https://github.com/everruns/everruns/tree/main/examples/support-agent)
is a complete Framework program. It builds an `Agent`, connects OpenAI,
attaches a typed lookup tool, creates a session through `Engine`, and answers a
real support question.

![Support Agent terminal demo](https://raw.githubusercontent.com/everruns/everruns/main/examples/support-agent/demo.gif)

This screencast replays a real run in readable pages, with waiting time removed.
[Read the complete displayed transcript](https://github.com/everruns/everruns/blob/main/examples/support-agent/demo.txt).
Tool-result previews are shortened; the agent receives the full tool response.

```bash
OPENAI_API_KEY=... cargo run -p everruns-support-agent
```

The included `cust_demo` record is deliberately non-sensitive. Supply a
different question after `--` to try another support flow. CI builds the agent
with a placeholder credential; running the binary makes the real model call.
