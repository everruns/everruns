---
title: Work Chat chooses who may sign in
description: Work Chat membership is open access, one person plus invites, or organization users. Sign-in follows that choice instead of a personal access token switch.
date: 2026-10-30
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/features/channels/#work-chat]
---

Who can use a Work Chat is a membership setting on the channel page.

**Open access** lets anyone register and sign in with the deployment's identity
provider. On Everruns Cloud that is an Everruns ID. **Single user + invites**
admits one email address plus the people listed as invites. **Organization
users** admits current members of the organization that owns the channel.
Someone who is not included is turned away at sign-in.

```json
{
  "membership": "org_users"
}
```

A Work Chat no longer shows the personal access token switch. An Agent API
channel still uses that switch for member tokens. A publisher-chosen identity
provider, such as external OpenID Connect, is not a membership mode yet.

See [Work Chat](/features/channels/#work-chat).
