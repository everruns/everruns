---
title: "Everruns SDKs record why a change was made"
description: "All three Everruns SDKs add with_reason, a derived client that sends a change reason the server stores in the changed entity's change history."
date: 2026-10-08
kind: feature
significance: notable
products: [sdk]
surfaces: [code]
release: "0.3.0"
links:
  docs: ["/features/change-history/", "/features/sdk/"]
  prs: [everruns/sdk#115]
---

Rust, Python and TypeScript clients gain `with_reason` (`withReason` in
TypeScript). It returns a derived client that sends the reason in the
`Everruns-Change-Reason` header with every request, and the server stores it in
the changed entity's history. The original client is unchanged, and a blank
reason sends no header.

```python
await client.with_reason("retire the unused agent").agents.delete("agent_123")
```

The server rejects a reason longer than 1000 characters, one with control
characters other than newline and tab, or one that looks like a credential,
with HTTP 400 `invalid_change_reason`.

See [Change history](/features/change-history/), [SDKs](/features/sdk/) and [#115](https://github.com/everruns/sdk/pull/115).
