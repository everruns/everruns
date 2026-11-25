---
title: "Retired Apps and their compatibility routes are removed"
description: "Use agent channels for public ingress: archival App reads, App-scoped URLs, App evaluation targets and App relationship counts are removed."
date: 2026-11-25
kind: breaking
significance: notable
products: [platform, cloud]
surfaces: [api, ui]
links:
  docs: [/features/channels/]
---

The retired Apps domain is removed. Use [agent channels](/features/channels/)
for public ingress. Existing channel IDs, hosted addresses, credentials and
conversation bindings remain intact. App-scoped URLs and archival App reads
now return 404; App relationship counts and the App eval target are gone.

```text
Before: POST /v1/apps/{app_id}/a2a/{channel_id}
After:  POST /v1/channels/{channel_id}/a2a
```

Invocation templates use agent and channel context; replace references to
`app` before upgrading. A2A clients using optional HMAC signatures must also use
the native channel ID for both signing-scope components. Self-hosted deployments must back up their database,
drain active turns and deliveries, and stop old API and worker processes before
applying this release's schema migration. Start the new API and workers together.
Historical migrations and immutable event records are retained.

The old Apps permissions are removed too. Custom permission resolvers should grant
`org:virtual-users:dangerous` for destructive virtual-user operations; the default
role mapping keeps those operations Owner-only.
