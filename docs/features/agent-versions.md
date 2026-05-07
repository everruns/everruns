---
title: Agent Versions
description: Save immutable agent snapshots, compare changes, roll back safely, and bind apps to a default, latest, or pinned agent version.
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
- Configure Apps to use the agent default, latest version, or a pinned version.

## Runtime Behavior

When a session is created, Everruns records the resolved `agent_version_id` on the session. Events emitted during that session include version metadata so logs, traces, and exports can identify the exact agent configuration that ran.

Apps can use:

- `default`: follow the agent default version.
- `latest`: always use the newest saved version.
- `pinned`: keep using a specific version until changed.

## Notes

Versions are immutable. Rollback updates the editable draft and saves a new rollback version so history remains append-only.
