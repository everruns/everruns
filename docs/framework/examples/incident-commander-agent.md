---
title: Incident Commander Agent
description: Coordinate a safe incident response with Meta Model API and a bounded status-update tool.
---

The [Incident Commander Agent source](https://github.com/everruns/everruns/tree/main/examples/incident-commander-agent)
uses Meta Model API's `muse-spark-1.3`. It records a concise incident update
before proposing impact assessment, owners, and safe next actions.

![Incident Commander Agent terminal demo](https://raw.githubusercontent.com/everruns/everruns/main/examples/incident-commander-agent/demo.gif)

This screencast replays a real run in readable pages, with waiting time removed.
[Read the complete displayed transcript](https://github.com/everruns/everruns/blob/main/examples/incident-commander-agent/demo.txt).
Tool-result previews are shortened; the agent receives the full tool response.

```bash
MODEL_API_KEY=... cargo run -p everruns-incident-commander-agent
```

Pass a different incident scenario after `--`. The tool refuses oversized
status updates and the instructions prohibit unsupported production claims.
`META_API_KEY` is accepted as an alternative environment variable.

Updates are appended to `incident.log` in the example folder, retained across
runs, and ignored by Git. No production infrastructure is modified.
