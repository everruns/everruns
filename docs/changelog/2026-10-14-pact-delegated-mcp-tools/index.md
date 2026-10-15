---
title: "PACT delegated turns discover the user's MCP tools"
description: "PACT delegated turns now discover account tools with the user's session grant, so scoped order lookups and approved cancellations reach the company API."
date: 2026-10-14
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [agent, api]
links:
  docs: [/features/a2a/#personal-agents-pact]
---

A [PACT delegated turn](/features/a2a/#personal-agents-pact) could hold a valid account token but fail to discover
its MCP tools when the customer had only a session grant. Discovery now uses
that grant's user identity for the configured server, so account tools are
available without creating a persistent connection.

The grant must belong to the user invoking the turn. Another user's grant,
an unattended turn, or a grant for another MCP server cannot supply that identity.
The company API continues to check the granted scope on each action.

The walkthrough shows order lookup with read-only permission, cancellation
refused before approval, and a verified signed receipt after approval. It uses a
local demo client and synthetic store data.

<iframe src="https://www.youtube-nocookie.com/embed/9FZiYv5PsWY?rel=0" title="PACT account consent and signed receipts in Everruns" loading="lazy" style="width:100%;aspect-ratio:16/9;border:0" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" referrerpolicy="strict-origin-when-cross-origin" allowfullscreen></iframe>

[Watch on YouTube](https://www.youtube.com/watch?v=9FZiYv5PsWY).
