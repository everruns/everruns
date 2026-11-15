---
title: "Tools in Shell is now Shell Code Mode"
description: "The capability that lets agents call their tools from Bashkit scripts is now shell_code_mode. Saved agents that use tools_in_shell keep working unchanged."
date: 2026-11-15
kind: improvement
significance: minor
products: [platform, cloud, framework]
surfaces: [agent, ui, code]
links:
  docs: [/capabilities/shell-code-mode/, /framework/code-mode/]
---

**Tools in Shell** is renamed to **Shell Code Mode** (`shell_code_mode`), the
name the rest of the industry uses for this pattern. Nothing changed in how it
works: the agent writes Bashkit scripts that call its tools through the `tools`
command and returns only what it needs.

Agents and Framework code that name `tools_in_shell` keep working. The old ID
resolves to the new capability, so nothing needs to be migrated. New agents and
code should use `shell_code_mode`:

```rust
use everruns::CapabilityRef;
use serde_json::json;

let code_mode = CapabilityRef::new("shell_code_mode").config(json!({"mode": "cli"}));
```

See [Shell Code Mode](/capabilities/shell-code-mode/) and
[Code Mode with MCP](/framework/code-mode/).
