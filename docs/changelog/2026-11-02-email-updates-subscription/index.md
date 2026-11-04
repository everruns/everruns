---
title: Email updates use one newsletter and honor topic opt-outs
description: The hosted demo uses Everruns updates, and the Profile switch reflects both topic-specific and global unsubscribe choices.
date: 2026-11-02
cloud: 2026-11-02
kind: fix
significance: minor
products: [cloud]
surfaces: [ui]
links:
  docs: [/getting-started/cloud/]
---

The [hosted demo](/getting-started/cloud/) now selects only **Everruns updates**
for new account enrollments and explicit email opt-ins. The separate Cloud
newsletter is retired. Existing unsubscribe
choices are preserved.

The **Email updates** switch in **Settings** > **Profile** now reflects an
Updates-topic opt-out as well as a global unsubscribe. Previously, a contact
who opted out of Updates but remained globally subscribed could still see the
switch turned on. If the provider cannot confirm the preference, the app
reports it as unavailable instead of assuming consent.
