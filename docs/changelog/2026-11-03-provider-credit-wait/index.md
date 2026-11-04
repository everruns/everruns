---
title: Turns out of provider credits retry smaller or wait for credits
description: When the provider refuses a request for lack of credits, the turn retries with fewer output tokens, or checks every minute for an hour and continues.
date: 2026-11-03
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [api]
links:
  docs: [/how-to/handle-errors-and-cancellation/, /event-reference/#turncredit_shortfall]
---

A turn the provider refused for lack of credits used to fail, even when the
balance covered most of the answer. Now, when the balance still covers at least
4,096 output tokens, the same request is retried once with max output tokens
lowered to what it covers.

Below that, the turn fails with `provider_quota_exhausted` and the session
checks for credits every minute for up to an hour. On OpenRouter a check reads
the key's balance without a model call. When credits are back the work
continues on its own; when the hour runs out it stops. See
[Handle errors and cancellation](/how-to/handle-errors-and-cancellation/).
Each step records a [`turn.credit_shortfall`](/event-reference/#turncredit_shortfall) event:

```json
{
  "type": "turn.credit_shortfall",
  "data": {
    "action": "wait_started",
    "requested_max_tokens": 65536,
    "affordable_tokens": 283,
    "next_check_at": "2026-11-03T14:05:05Z"
  }
}
```
