---
title: Auto approval lets a reviewer model answer routine tool calls
description: Tool Approval has a new auto mode. A reviewer model allows routine calls, blocks risky ones with a reason, and hands anything unclear to a person.
date: 2026-11-17
kind: feature
significance: notable
products: [platform, cloud, framework, cli]
surfaces: [ui, api, cli, code]
links:
  docs: [/capabilities/tool-approval/#auto-mode, /capabilities/soft-approval/]
---

A four-hour refactor no longer sits idle because someone stepped away from an
approval card. Set [Tool Approval](/capabilities/tool-approval/#auto-mode) to
`auto` and a reviewer model answers first: a branch push you asked for runs, a
`curl` that uploads `.env` to a paste site is blocked with the rule it broke,
and anything unclear still waits for you.

The reviewer reads what people wrote and what the agent did, never tool output,
so a hidden instruction in a fetched page cannot argue past it. When it is down,
a person decides. After a block the agent asks you through
[soft approval](/capabilities/soft-approval/) and retries.

```bash
everruns sessions approval-mode set --session-id $SESSION_ID --mode auto
```

Switch a session from its header. Agents add `allow`, `ask` or `never` rules;
Settings → Approvals limits the organization's modes and lists recent verdicts;
My approvals sets your default mode and stricter rules. Chat marks auto-allowed
and blocked calls.
