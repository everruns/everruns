---
title: Hard approvals preserve the full command
description: Yolop rejects oversized approval text, preserves complete commands through thread relays, and denies requests from older hosts after an upgrade.
date: 2026-10-12
kind: fix
significance: notable
products: [yolop]
surfaces: [cli, operations]
links:
  docs: [https://github.com/everruns/yolop/blob/main/docs/features/approvals.md]
  prs: [everruns/yolop#847]
---

Hard approvals preserve the complete command and reason through the thread
relay. Commands or reasons above 32 KiB are denied rather than shortened for
review, including after a session grant.

Ask the agent to split oversized work into smaller commands. After upgrading,
restart older thread hosts and submit pending approval requests again.
Approval once and approval for the session keep their existing sandbox scopes.

See [Yolop approvals](https://github.com/everruns/yolop/blob/main/docs/features/approvals.md)
for the hard approval limits and controls.
