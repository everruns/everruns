---
title: Connect customer-hosted agents over HTTPS
description: Register your own agent runtime with an HTTPS endpoint and keep Everruns sessions, tool traces, conversation history and Slack Direct replies.
date: 2026-10-28
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api, operations]
links:
  docs: [/how-to/cloudflare-agent/]
---

![Everruns plus Cloudflare Workers](./integration@2x.png)

![External worker execution settings on an ordinary Agent](./execution@2x.png)

Connect an agent running in your own infrastructure through **Configure execution**
on its ordinary Agent page. Verify an HTTPS endpoint with a shared secret, then
use the same Playground, sessions and Trace views. Disabling the endpoint keeps
queued work bound to it.

The [Cloudflare Agent guide](/how-to/cloudflare-agent/) includes a complete
JavaScript agent with a real model, a private D1 database tool, registration and
normal API invocation. Slack channels use Direct communication for basic replies.
The endpoint supplies model and tool instrumentation; native approvals,
delegation and Explicit communication are outside this example.

Self-hosted installations enable `EXTERNAL_AGENTS_ENABLED=true`. Cloud availability
requires the corresponding hosted deployment.
