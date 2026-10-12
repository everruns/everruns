---
title: "Cloudflare Web Search joins as a Web Search provider"
description: "Connect a Cloudflare account as a search account and the web_search tool searches Ceramic, Exa or Linkup through the Cloudflare Web Search API."
date: 2026-10-12
kind: feature
significance: minor
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/capabilities/web-search/, /integrations/cloudflare-web-search/]
  prs: [everruns/everruns#4643]
---

[Cloudflare Web Search](/integrations/cloudflare-web-search/) is now a Web Search provider next to
Brave Search and Parallel. Connect a Cloudflare account in **Registers > Web search** with an API
token, the account ID and the index to search: Ceramic ($0.25 per 1,000 searches, zero data
retention), Exa or Linkup. Searches run through your AI Gateway, so they appear in its logs, and
bill to its credits or to an Exa or Linkup key you stored there.

Agents keep their one `web_search` tool. Pick the account in an agent's Web search settings or
place it in the Automatic order, and a failed search falls through to the next provider as usual.

Connecting through the API takes the same fields as the form:

```bash
curl -X POST "$EVERRUNS_URL/v1/orgs/$ORG/organization-connections/providers/cloudflare_web_search" \
  -H "Authorization: Bearer $EVERRUNS_TOKEN" -H "Content-Type: application/json" \
  -d '{"name": "Team", "api_key": "'"$CLOUDFLARE_API_TOKEN"'",
       "account_id": "'"$CLOUDFLARE_ACCOUNT_ID"'", "index": "ceramic"}'
```

See [Web Search](/capabilities/web-search/).
