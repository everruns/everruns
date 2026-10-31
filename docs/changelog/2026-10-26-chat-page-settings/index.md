---
title: "Review Chat changes across app pages"
description: "Shared Before/After review, guarded Save and Undo across supported page editors, with atomic model selection and private profile history."
date: 2026-10-26
kind: improvement
significance: notable
products: [platform, cloud]
release: "0.50.0"
surfaces: [ui]
links:
  docs: [/built-ins/harnesses/platform-chat/]
  prs: [everruns/everruns#4920]
---

Enable **Chat workspace** in Adoption to work beside app pages. Chat reads live
forms and stages edits; **Save** or **Create** stays yours. Existing-resource editors
share **Before/After**, guarded **Undo** and **History** across navigation. Model
selections apply atomically; private profile history stays owner-only.

Knowledge tabs provide collection, source, search and file context.
Translations, tracing, interface language, trigger edits and notes support drafts.
Collection Undo preserves other settings. Protected controls stay manual.

**New subchat** starts an independent conversation. Full Chat shows one conversation;
Command/Ctrl-J opens it beside an app page. Core Chat and subchats need no flag.
**Chat background work** independently enables delegation and its Work panel.

See [Platform Chat](/built-ins/harnesses/platform-chat/).

## Update 2026-10-28

Long dialogs scroll, keeping Save reachable.

## Update 2026-10-29

Channel drafts become editable after selecting a type; returning to the chooser expires them.
Cost-planner drafts update estimates locally and support Undo.

![A fresh subchat opens one conversation](./subchat@2x.png)

![Review the live draft before Save](./settings-draft@2x.png)

![Saved changes retain Before/After and Undo](./saved-review@2x.png)
