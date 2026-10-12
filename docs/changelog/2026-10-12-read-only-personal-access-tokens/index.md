---
title: "Create read-only personal access tokens"
description: "A personal access token can now be read-only: it reads every organization you belong to, and is refused anything that creates, changes or runs."
date: 2026-10-12
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/features/ui/#token-access]
  prs: [everruns/everruns#4623, everruns/everruns#4613, everruns/everruns#4598, everruns/everruns#4583]
visual: "Create personal access token dialog with the Full access and Read-only choices (/settings/personal-access-tokens), cropped to the dialog"
---

**In the UI:** open **Settings** > **Personal access tokens**, create a token,
and pick **Read-only** instead of **Full access**. The token list shows each
token's access.

**In the API:** pass `"scopes": ["read"]` when creating a token. A read-only
token has read access to all your organizations, so treat it like a password.
It cannot create, change, run or delete anything, manage tokens, or download
the full account export; those requests fail with `403` and
`"code": "insufficient_scope"`. `GET /v1/auth/me` reports the token's
`token_scope`.

See [Token access](/features/ui/#token-access).
