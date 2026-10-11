---
title: "Sign in to channels and Public Chat with AgentID"
description: "A2A, AG-UI and Public Chat channels accept AgentID tokens, Public Chat shows Continue with AgentID, and agents can sign in to apps with their AgentMail inbox."
date: 2026-10-07
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [agent, ui, operations]
release: "0.44.0"
links:
  docs: [/features/channels/#sign-in-with-agentid, /integrations/agentid/]
  prs: [everruns/everruns#4326, everruns/everruns#4334, everruns/everruns#4337]
---

Channels can accept [AgentID](https://www.agentid.com), a sign-in for AI agents.
A2A, AG-UI and Public Chat channels take AgentID bearer tokens, and each agent
that signs in becomes its own end user in the organization.

To let agents sign in to a Public Chat channel, register an AgentID client, set
`AGENTID_CLIENT_ID` and `AGENTID_CLIENT_SECRET` on the server, and set the
channel's sign-in to **AgentID**. The chat page then shows **Continue with
AgentID**.

Going the other way, the experimental **AgentID sign-in** capability gives an
agent the `agentid_authorize` tool, which approves an app's AgentID sign-in for
the agent's AgentMail inbox.

See [Sign in with AgentID](/features/channels/#sign-in-with-agentid),
[AgentID](/integrations/agentid/) and [#4326](https://github.com/everruns/everruns/pull/4326), [#4334](https://github.com/everruns/everruns/pull/4334), [#4337](https://github.com/everruns/everruns/pull/4337).
