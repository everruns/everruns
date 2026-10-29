---
title: Shared controls across resource pickers and forms
description: Search models, agents and harnesses with the same picker, use shared form controls, and reach organization settings through compact mobile navigation.
date: 2026-10-27
kind: improvement
significance: minor
products: [platform, cloud]
surfaces: [ui]
cloud: 2026-10-28
links:
  docs: [/design-system/components/]
---

Model, agent and harness pickers now share searchable selection, including account names,
harness descriptions and unavailable saved values. Observer forms use searchable resource
bindings, tag chips and a styled sampling slider. Reports and provider pages use the shared
page layout, and mobile Settings replaces its full navigation list with a destination selector.

Destructive actions use application confirmation dialogs. Leaving the page or switching
organizations cancels unanswered confirmations. Shared pagination, pending buttons and form
feedback follow the same [Slate component rules](/design-system/components/).

![Observer creation with shared fields and searchable resource bindings on mobile](./observer-form@2x.png)
