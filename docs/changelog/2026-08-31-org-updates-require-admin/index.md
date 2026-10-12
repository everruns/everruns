---
title: Only organization admins can rename an organization
description: Every PATCH /v1/orgs/{org} request now requires an organization admin, closing a path where members could rename non-default organizations.
date: 2026-08-31
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [api, operations]
release: "0.24.0"
links:
  prs: [everruns/everruns#3315]
---

Organization updates now check for an admin on every request. Before, a name-only `PATCH /v1/orgs/{org}`, the request the settings page autosave sends, skipped the admin check, so a non-admin member could rename a non-default organization. Such a request now returns `403` and nothing is saved.

Admins see no change. See [#3315](https://github.com/everruns/everruns/pull/3315).
