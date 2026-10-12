---
title: Send a message and wait for the turn result in one request
description: The sessions API can now hold a message request open until the turn finishes, and Framework runs accept a timeout, for callers that do not consume SSE.
date: 2026-09-12
kind: feature
significance: notable
products: [platform, cloud, framework]
surfaces: [api, code]
release: "0.26.0"
links:
  docs: [/framework/sessions/, /how-to/stream-events/]
  prs: [everruns/everruns#3541]
---

Callers that want a result instead of an event stream can now wait for it.

**In the API:** `POST /v1/sessions/{id}/messages?wait=true` returns `200` with the
turn result when the turn completes or fails, or `202` with partial messages when
`timeout_ms` (default 120 s, at most 600 s) expires. Without `wait` it still
returns `201` immediately, so existing clients are unaffected.

```bash
curl -X POST "http://localhost:9300/api/v1/sessions/$SESSION_ID/messages?wait=true&timeout_ms=60000" \
  -H "Authorization: Bearer $EVERRUNS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"message": {"role": "user", "content": [{"type": "text", "text": "Hello!"}]}}'
```

**In code:** `RunOptions::timeout` bounds `Session::run_with`; when it expires the
in-process turn is cancelled and its outcome returned.

See [Sessions](/framework/sessions/) and [Stream events](/how-to/stream-events/). [#3541](https://github.com/everruns/everruns/pull/3541)
