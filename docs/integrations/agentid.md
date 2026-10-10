---
title: AgentID Sign-In
description: Let an agent sign in to apps that support AgentID as itself, using an AgentMail inbox connected to the agent's service account.
appliesTo: [platform]
---

> **Status:** Experimental. No feature flag: adding the capability and an AgentMail connection is the opt-in.

[AgentID](https://www.agentid.com) is a sign-in for AI agents, run by AgentMail. An agent's AgentID is an AgentMail inbox. When an app that supports AgentID asks an agent to sign in, it shows a waiting page with a short-lived auth token; the agent's inbox approves it, and the app sees the agent signed in.

The **AgentID sign-in** capability gives an Everruns agent the `agentid_authorize` tool, which does that approval with the agent's own inbox.

This page covers agents signing in to other apps. To let agents sign in to your own channels, see [Channels: Sign in with AgentID](/features/channels/#sign-in-with-agentid).

## Setup

1. Create an inbox for the agent in AgentMail, register it with AgentID, and create an AgentMail API key.
2. Open the agent, then its **service account** connections, and connect **AgentMail** with the API key and the inbox address (for example `research@acme.agentmail.to`).
3. Add the **[Experimental] AgentID sign-in** capability to the agent.

The connection belongs to the agent. People chatting with the agent never get it, and the tool never uses their own connections or an administrator's.

## Tool

`agentid_authorize` takes one argument:

| Parameter | Description |
|---|---|
| `auth_token` | The auth token from the app's AgentID waiting page |

It asks AgentMail to approve that sign-in for the agent's inbox, accepting AgentID's disclosure of the agent's identity to the app. On success it returns `authorized: true`, the inbox, and AgentMail's `api_key_id` and `instructions`. If AgentMail refuses (for example the agent has reached its app limit, or the token has expired), the tool returns AgentMail's reason.

The tool sends nothing else to AgentMail: it does not send or read mail.

## Security

- The AgentMail key is read only from the agent's service account. With no AgentMail connection there, the tool refuses.
- Requests go only to `https://api.agentmail.to`, through the agent's network access policy.
- Approving a sign-in shares the agent's AgentID with the app. Only pass a token that the app showed the agent.
