---
title: "Create read-only personal access tokens"
description: "A personal access token can now be read-only: it reads every organization you belong to, and is refused anything that creates, changes or runs."
date: 2026-10-12
kind: feature
significance: notable
products: [platform, cloud, cli]
surfaces: [ui, api, cli]
release: "0.48.0"
links:
  docs: [/features/ui/#token-access, /features/cli/#configure]
  prs: [everruns/everruns#4659, everruns/everruns#4626, everruns/everruns#4623, everruns/everruns#4613, everruns/everruns#4598, everruns/everruns#4583]
---

**In the UI:** open **Settings** > **Personal access tokens**, create a token,
and pick **Read-only** instead of **Full access**. The token list shows each
token's access.

**In the API:** pass `"scopes": ["read"]` when creating a token:

```bash
curl -X POST "$EVERRUNS_URL/api/v1/auth/personal-access-tokens" \
  -H "Authorization: Bearer $SESSION_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name": "ci-reader", "scopes": ["read"], "expires_in_days": 90}'
```

A read-only
token has read access to all your organizations, so treat it like a password.
It cannot create, change, run or delete anything, manage tokens, or download
the full account export; those requests fail with `403` and
`"code": "insufficient_scope"`. `GET /v1/auth/me` reports the token's
`token_scope`.

**In the CLI:** [`everruns login --token`](/features/cli/#configure) accepts a read-only token,
`everruns whoami` shows its access, and a refused command says the token is
read-only.

See [Token access](/features/ui/#token-access).
