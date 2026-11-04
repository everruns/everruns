---
title: "Fixes for triggered sandboxes, blocked tools and agent prompts"
description: "Trigger-started sessions use the organization sandbox account, tools blocked on a connection report a failure, and agent prompts lose the generic opener."
date: 2026-10-12
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [ui, api, agent]
links:
  docs: [/features/sandbox-templates/, /built-ins/harnesses/base/]
---

A session started by a trigger has no user, so a Daytona or E2B template set to
the **Personal** provider account failed to start. It now uses the organization's
provider account when there is exactly one, as described in
[Sandbox Templates](/features/sandbox-templates/).

The **Provider account** picker now says in plain words whose connection each
choice uses and who pays, labels every account with its provider, such as
**Daytona · Personal** or **Daytona · Reviews**, and links to **Provider
accounts** when the organization has no account for that provider yet.

A tool call stopped by a missing connection now fails instead of returning an
empty success, so the agent reads that it did not run:

```json
{
  "connection_required": "daytona",
  "error": "This tool did not run: its daytona connection is not set up. The person has to connect daytona, then try again."
}
```

The [Base harness](/built-ins/harnesses/base/) no longer puts "You are a helpful
assistant." ahead of an agent's own prompt.
