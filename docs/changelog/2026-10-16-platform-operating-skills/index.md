---
title: Everruns shares product and workflow skills
description: Find product concepts, CLI examples, agent-authoring workflows, and Rust guidance through shared skills in Everruns and its coding-agent plugin.
date: 2026-10-16
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [agent]
links:
  docs: [/capabilities/platform/, /getting-started/use-in-ai-tools/]
---

The general `everruns` skill now introduces product concepts, documentation,
and companion workflows. `everruns-cli` covers command usage and examples;
`everruns-framework` holds Rust development guidance. The existing MCP and
agent-authoring guides remain focused on their workflows. The MCP guide covers
connection setup and lists the four skills exposed by the MCP server, with
detailed [Cloud authentication guidance](https://everruns.com/auth.md).

The [Platform capability](/capabilities/platform/) supplies the general, CLI,
and authoring skills as read-only files loaded on demand:

```json
{
  "name": "everruns-cli"
}
```

Pass this argument to `activate_skill`. Both Platform interfaces receive the
same skills. MCP clients can discover the general and CLI guides alongside the
existing MCP workflows. The [coding-agent plugin](/getting-started/use-in-ai-tools/)
distributes all five shared skills, including Framework references, and keeps
its additional task guides for running, debugging, evaluating, and shipping
agents.
