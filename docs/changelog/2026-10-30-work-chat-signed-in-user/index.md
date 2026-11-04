---
title: Work Chat shows the signed-in person
description: The Work Chat sidebar shows the signed-in person's name, plus email and picture when the identity provider includes them.
date: 2026-10-30
kind: improvement
significance: minor
products: [platform, cloud]
surfaces: [ui]
links:
  docs: [/features/channels/#work-chat]
---

The Work Chat sidebar now shows who is signed in. The name comes from their
identity provider, with their email and picture when the sign-in includes
them. Sign out stays beside that profile.

A provider that does not send a name leaves the Work Chat label in place.
Pictures are limited to https addresses, and loading one does not send the
chat address to the image host.

See [Work Chat](/features/channels/#work-chat).

![Work Chat sidebar with the signed-in person's name and email](./account@2x.png)
