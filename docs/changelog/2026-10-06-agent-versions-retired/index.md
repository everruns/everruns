---
title: "Agent versions are retired in favor of change history"
description: "Agent publishing, default versions and version pins are removed; channels, triggers and sessions now run the agent's current configuration instead."
date: 2026-10-06
kind: breaking
significance: notable
products: [platform, cloud, cli]
surfaces: [api, ui, cli]
release: "0.44.0"
links:
  docs: [/features/change-history/#agent-revisions-replace-agent-versions]
  prs: [everruns/everruns#4312]
---

Agent versions, with publishing, semver, a default version and version pins,
are gone. A migration copies every saved version into the agent's change
history as a revision. Channels, triggers, apps and session participants now
run the agent's current configuration, and each new session records the
`agent_revision` it started on.

Version endpoints, CLI verbs and fields such as `agent_version_id` and
`default_version_id` are removed. To go back to an earlier configuration, use
**History → Restore this point** on the agent page or `everruns history restore`.

See [Agent revisions replace agent versions](/features/change-history/#agent-revisions-replace-agent-versions)
and [#4312](https://github.com/everruns/everruns/pull/4312).
