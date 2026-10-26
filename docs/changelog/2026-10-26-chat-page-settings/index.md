---
title: "Review Chat changes across app pages"
description: "Shared Before/After review, guarded Save and Undo across supported page editors, with atomic model selection and private profile history."
date: 2026-10-26
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [ui]
links:
  docs: [/built-ins/harnesses/platform-chat/]
  prs: [everruns/everruns#4920]
---

With **Chat mode** enabled in Adoption, Chat can stage settings, schedule and
eval-case fields, invitations, MCP metadata, report choices and Playground setup.
Organization, Model defaults and Web search defaults use **Save** and **Discard**.
Review the draft before saving; dirty dialogs preserve edits on refresh and block
conflicting saves. Credentials and approvals retain manual controls.

Existing-resource editors also share **Before/After**, field-level **Undo**,
**Undo all** and **History**. Save and Undo check the resource revision, so a newer
edit cannot be overwritten silently. Reviews remain available while navigating
within the organization. Model selection applies and undoes atomically. Account
names and Sandbox Template metadata preserve current credentials and execution
settings; account profile history is private to its owner.

Operational pages share bounded status rows for queues, workers, workflows,
schedules, circuit breakers and Monitoring. Settings navigation and grids adapt
to the space beside Chat. The organization opt-in remains unchanged.

See [Platform Chat](/built-ins/harnesses/platform-chat/).

![Organization settings keep edits in a draft until Save](./settings-draft@2x.png)

![Saved review keeps Before/After and guarded Undo available](./saved-review@2x.png)
