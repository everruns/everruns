---
title: Everruns Support Agent
description: Troubleshoot Everruns Framework questions with Claude Opus and authoritative documentation links.
---

The [Everruns Support Agent source](https://github.com/everruns/everruns/tree/main/examples/everruns-support-agent)
shows a real Anthropic-backed troubleshooting agent. Its typed tool returns the
relevant Everruns documentation links before the model explains the smallest
safe next step.

![Everruns Support Agent terminal demo](https://raw.githubusercontent.com/everruns/everruns/main/examples/everruns-support-agent/demo.gif)

```bash
ANTHROPIC_API_KEY=... cargo run -p everruns-framework-support-agent
```

It uses `claude-opus-5`. Pass a different question after `--` to use the same
agent for another Framework support issue.
