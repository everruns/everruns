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

Enable **Chat workspace** in Adoption to work beside app pages. Chat reads live
forms and stages edits; **Save** or **Create** stays yours. Existing-resource editors
share **Before/After**, guarded **Undo** and **History** across navigation. Model
selections apply atomically; private profile history stays owner-only.

Knowledge tabs provide current collection, source, search and visible-file context.
Agent/Harness translations join saved review; interface language, trigger quick
edits and manager notes support staged changes. Conflicting drafts stay intact.
Tracing routing joins review; collection field Undo preserves other settings.
Protected sheets, credentials and approvals retain their manual controls.

**New subchat** starts an independent conversation. Full Chat shows one conversation;
Command/Ctrl-J opens it beside an app page. Core Chat and subchats need no flag.
**Chat background work** independently enables delegation and its Work panel.

See [Platform Chat](/built-ins/harnesses/platform-chat/).

## Update 2026-10-28

Long dialogs scroll within the page pane, keeping Save reachable after draft changes.

## Update 2026-10-29

New channel drafts become editable in Chat after you select a channel type.
Returning to the type chooser clears the previous draft binding.

![A fresh subchat opens one conversation](./subchat@2x.png)

![Review the live draft before Save](./settings-draft@2x.png)

![Saved changes retain Before/After and Undo](./saved-review@2x.png)
