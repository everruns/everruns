---
title: "ScriptedTool accepts clap-based builtins"
description: "ScriptedToolBuilder::builtin registers raw-argv builtins next to ToolDef tools, and closed schemas now reject unknown flags with an error naming valid ones."
date: 2026-09-17
kind: feature
significance: minor
products: [bashkit]
surfaces: [code]
release: "0.18.1"
links:
  docs: ["https://github.com/everruns/bashkit/blob/main/docs/scripted-tools.md"]
  prs: [everruns/bashkit#2408]
---

`ScriptedToolBuilder::builtin(...)` registers a builtin that receives raw
arguments, next to the `ToolDef` tools of a `ScriptedTool`. Commands that need
positionals, short flags, `--` or required-argument checks at parse time can
now use clap inside a `ScriptedTool`:

```rust
.builtin("get_agent", Box::new(GetAgent))
```

A tool whose schema sets `"additionalProperties": false` now rejects an unknown
flag with an error that names the valid flags, instead of silently dropping it.
clap output is stripped of ANSI codes.

See [Scripted tools](https://github.com/everruns/bashkit/blob/main/docs/scripted-tools.md), [clap builtins](https://github.com/everruns/bashkit/blob/main/crates/bashkit/docs/clap-builtins.md) and [#2408](https://github.com/everruns/bashkit/pull/2408).
