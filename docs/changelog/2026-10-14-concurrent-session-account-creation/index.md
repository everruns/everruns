---
title: Concurrent session creation reuses one runtime account
description: Simultaneous first sessions for the same management account now reuse one runtime identity, avoiding intermittent conflicts during session creation.
date: 2026-10-14
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [api]
---

Creating several sessions at once for a management account's first use in an
organization now reuses the same runtime account. Previously, competing requests
could return HTTP 409 with `already_exists` while creating that account's default
identity binding, even though each request was creating a separate session.

No client changes are required. Each successful request still creates its own
session, while the runtime account remains scoped to the same organization and
verified identity. Revoked identity bindings remain unavailable.
