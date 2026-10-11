---
title: "Bashkit adds an in-process terminal and TerminalTool for agents"
description: "A terminal feature runs a Bashkit session in an in-memory terminal with vi, less and more, and TerminalTool lets an LLM agent type keys across named tabs."
date: 2026-10-07
kind: feature
significance: major
products: [bashkit]
surfaces: [code, agent]
links:
  docs: ["https://github.com/everruns/bashkit/blob/main/docs/terminal.md"]
  prs: [everruns/bashkit#2509, everruns/bashkit#2518, everruns/bashkit#2521, everruns/bashkit#2523, everruns/bashkit#2548]
---

The off-by-default `terminal` feature wraps a Bashkit session in an in-memory
terminal with `vi` and interactive `less` and `more`. `TerminalTool` turns it
into one agent tool call: the model sends keys in Vim notation and gets back
the screen, the activity (`prompt`, `continuation`, `running` or `exited`) and
the commands that finished.

```rust
let mut tool = TerminalTool::new(Bash::builder());
let out = tool.call(json!({"input": "vi /tmp/n.txt<Enter>"})).await?;
```

`TerminalTool::with_sessions` adds up to 8 named tabs that share the filesystem
and keep running in the background. Python (`bashkit.Terminal`) and Node
(`Terminal` in `@everruns/bashkit`) expose the same calls.

See [Terminal](https://github.com/everruns/bashkit/blob/main/docs/terminal.md), [#2509](https://github.com/everruns/bashkit/pull/2509), [#2518](https://github.com/everruns/bashkit/pull/2518), [#2521](https://github.com/everruns/bashkit/pull/2521), [#2523](https://github.com/everruns/bashkit/pull/2523) and [#2548](https://github.com/everruns/bashkit/pull/2548).
