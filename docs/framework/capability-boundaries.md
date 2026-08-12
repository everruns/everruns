---
title: Portable and hosted capabilities
description: Understand which capabilities run in the Framework and which require the Everruns Platform host.
---

The Framework advertises only capabilities that can run through its portable,
in-process host contract. A capability reference remains an open value, so an
application may retain configuration for an ID supplied by another host or
plugin, but an unknown or hosted-only ID contributes no prompt, tools, or
behavior in the default Framework runtime.

## Portable Framework capabilities

Portable built-ins use services available from the Framework runtime or from
an explicit application integration. Examples include files, session storage,
current time, compaction, tool search, skills, and application-authored
capabilities. Register application behavior with `#[everruns::tool]` or
`everruns::capability` rather than depending on product internals.

## Hosted Platform capabilities

Knowledge Bases and Knowledge Indexes, Memories, subagents and agent handoff,
background/session tasks and schedules, user hooks, model scouting,
OpenRouter workspace management, citations, and platform-management tools need
hosted persistence or orchestration. Their implementations and narrow service
contracts live in `everruns-platform`; the server and worker product presets
register them explicitly.

This boundary prevents the public Framework from promising tools whose stores,
tenant scope, workers, or authorization services are absent. It does not change
persisted capability IDs or JSON configuration. A specialized low-level host
can depend on `everruns-platform`, install the required services, and select the
hosted registry deliberately.

For application-owned behavior, continue with
[advanced capabilities](/framework/advanced-capabilities/). For low-level host
composition, see [custom backends](/framework/custom-backends/).
