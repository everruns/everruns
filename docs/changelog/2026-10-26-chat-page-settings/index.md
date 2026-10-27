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

Enable **Chat workspace** in Adoption to work beside app pages. Chat reads supported
live forms, stages edits, and leaves **Save** or **Create** to you. Credentials and
approvals keep their manual controls.

Existing-resource editors share **Before/After**, field-level **Undo**, **Undo all**
and **History**. Save and Undo reject stale revisions. Reviews survive navigation;
model selections apply and undo atomically. Private profile history stays owner-only.

Connected-account names, Sandbox Template labels, and your organization profile
share the guarded review flow through REST and Chat.

**New subchat** starts an independent conversation on the same assistant. Selecting
it replaces the conversation. Full Chat shows one conversation; Command/Ctrl-J
opens page-side Chat from an app page. Core Chat and subchats need no feature flag.
**Chat background work** separately enables delegation and its Work panel.

See [Platform Chat](/built-ins/harnesses/platform-chat/).

![A fresh subchat opens one conversation](./subchat@2x.png)

![Review the live draft before Save](./settings-draft@2x.png)

![Saved changes retain Before/After and Undo](./saved-review@2x.png)
