---
title: Auto approval lets a reviewer model answer routine tool calls
description: Tool Approval has a new auto mode. A reviewer model allows routine calls, blocks risky ones with a reason, and hands anything unclear to a person.
date: 2026-11-17
kind: feature
significance: notable
products: [platform, cloud, framework, cli]
surfaces: [api, cli, code]
links:
  docs: [/capabilities/tool-approval/#auto-mode, /capabilities/soft-approval/]
---

A four-hour refactor no longer sits idle because someone stepped away from an
approval card. Set [Tool Approval](/capabilities/tool-approval/#auto-mode) to
`auto` and a reviewer model answers first: a branch push you asked for runs, a
`curl` that would upload `.env` to a paste site is blocked with the rule it
broke, and anything it is unsure about still waits for you.

The reviewer reads what people wrote and what the agent did, never tool output
or the agent's own text, so an instruction hidden in a fetched page cannot
argue its way past it. When it is down it asks a person instead of allowing.
After three blocks in a row it stops reviewing and hands every call to you.

```bash
everruns sessions approval-mode set --session-id $SESSION_ID --mode auto
```

Agents can add `rules` (`allow`, `ask` or `never` per tool pattern) and trust
notes; organizations can limit which modes are allowed with
`tool_approval_policy`; each person can set a default mode and stricter rules
of their own. Every verdict lands in the session log and the organization audit
log.
