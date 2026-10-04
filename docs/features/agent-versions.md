---
title: Agent Versions
description: Save immutable Agent snapshots, compare changes, roll back, and bind channels and triggers to a default, latest, or pinned version.
appliesTo: [platform, cloud]
---

# Agent Versions

Agent versions are saved snapshots of an agent configuration. They let you preserve a known-good prompt, tool, and capability setup while continuing to edit the draft agent.

This feature is gated by `FEATURE_AGENT_VERSIONS`.

## What You Can Do

- Save the current agent draft as a new version.
- Set a default version for normal use.
- Compare authored and resolved configuration between two versions.
- Roll back the editable draft to a previous version.
- Fork a version into a new agent.
- Configure each endpoint and trigger to use the Agent default, latest version, or a pinned version.

## Runtime Behavior

When a session is created, Everruns records the resolved `agent_version_id` on the session. Events emitted during that session include version metadata so logs, traces, and exports can identify the exact agent configuration that ran.

Channels and triggers can use:

- `default`: follow the agent default version.
- `latest`: always use the newest saved version.
- `pinned`: keep using a specific saved version until changed, even after the agent is edited or its default moves.

Set it in the **Agent version** section of the channel or trigger editor, reached from the agent's **Integrations** tab. Pinned exposures show a pin badge there. Through the API, send `agent_version_policy` (and `agent_version_id` when pinning) on `POST`/`PATCH /v1/agents/{agent_id}/channels` and `/v1/agents/{agent_id}/triggers`; switching back to `default` clears the pin. Only saved versions of the same agent can be pinned.

## Notes

Versions are immutable. Rollback updates the editable draft and saves a new rollback version so history remains append-only.
