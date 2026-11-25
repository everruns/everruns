---
title: MCP event webhooks follow the final Events contract
description: MCP event subscriptions use final extension discovery, required routing headers and delivery errors while preserving signed webhook delivery.
date: 2026-11-25
kind: fix
significance: minor
products: [platform]
surfaces: [api]
links:
  docs: [/features/mcp/]
---

MCP clients can discover session webhooks through the final Events extension
identifier. Named subscription requests carry the required event routing header,
and callback failures use the final protocol error codes. A callback's `410 Gone`
now abandons only that delivery; other events keep reaching the subscription.

Agent event triggers check extension support before subscribing, mirror annotated
subscription arguments into safe headers, and refresh short grants before expiry.
Webhook signatures and deduplication by delivery ID remain in place. Poll, push
and replay are unavailable. See [MCP integration](/features/mcp/) for subscriptions
and agent triggers.

Each delivered message has an independent ID, stable across its retries, so the
same event on two subscriptions reaches both. Callback consent is reused for
the same principal and URL across refreshes and filters. An omitted cursor
clears an earlier replay position just like an explicit null.
