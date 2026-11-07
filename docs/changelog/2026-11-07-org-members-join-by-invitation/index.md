---
title: Organization members now join only by invitation
description: Org admins can no longer add an existing user by ID, and callers that name no organization now default to the one they joined first.
date: 2026-11-07
kind: breaking
significance: notable
products: [platform, cloud]
surfaces: [api, agent]
links:
  docs: [/features/ui/]
---

Organization owners and admins add people by invitation, and the invitee
accepts with a verified matching email. `POST /v1/orgs/{org}/members`, which
added an existing user by ID without asking them, now returns `403` unless the
caller is a platform admin. Scripts that used it should send an invitation
instead (see [Members](/features/ui/)):

```sh
# Before: added the user immediately
curl -X POST "$EVERRUNS_URL/v1/orgs/$ORG/members" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"user_id": "0193...", "role": "member"}'
# After: the user accepts the returned invite_url
curl -X POST "$EVERRUNS_URL/v1/orgs/$ORG/invites" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"email": "teammate@example.com", "role": "member"}'
```

MCP clients and other callers that send no organization header or cookie now
land in the organization the user joined first, not the one whose name sorts
first. The organization list follows the same order.
