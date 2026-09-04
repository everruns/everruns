---
title: Coding Review Agent
description: Review a self-contained change with Claude Sonnet and a typed code-inspection tool.
---

The [Coding Review Agent source](https://github.com/everruns/everruns/tree/main/examples/coding-review-agent)
uses `claude-sonnet-5` to inspect and review the included
`sample_payment.rs`. The agent must read the change through its typed tool and
then report only material, reproducible findings.

![Coding Review Agent terminal demo](https://raw.githubusercontent.com/everruns/everruns/main/examples/coding-review-agent/demo.gif)

```bash
ANTHROPIC_API_KEY=... cargo run -p everruns-coding-review-agent
```

Pass a different review instruction after `--` to change the review focus.
