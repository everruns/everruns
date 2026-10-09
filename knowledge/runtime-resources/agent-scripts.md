---
type: Specification
title: "Agent Scripts"
description: "Agent scripts (saved shell scripts an agent owns; the resource behind tools in shell saved scripts)."
tags:
  - everruns
  - runtime-resources
---
# Agent Scripts

## Abstract

An **agent script** (user-facing name: *saved script*) is a shell script an
agent owns: a name, a one-line description, an optional input schema, and the
body. It is the storage half of
[tools in shell](../execution/tools-in-shell.md) section D8; how a script is
called, saved from the shell, and permissioned lives there. This concept covers
only the resource.

It is modeled on [agent triggers](agent-triggers.md) and deliberately much
smaller: no schedule, webhook, or delivery machinery. Nothing runs on its own.

Field shapes, limits, SQL, and handlers live in code: `AgentScript` in
`crates/server/src/records/agent_script.rs`, migration
`194_agent_scripts.sql`, the `crates/server/src/domains/agent_scripts/` domain
(limits in `validation.rs`), and the `/v1/agents/{agent_id}/scripts` API.

## Model

- Org-scoped, owned by one agent; ids are typed (`scr_`).
- The name is unique among the agent's **active** scripts and immutable.
  Delete archives the script (same `active` / `archived` / `deleted` lifecycle
  as triggers) and frees the name.
- An agent holds a bounded number of active scripts; name, description, body
  and schema are bounded, and the schema must describe an object.
- List returns full records including bodies, because the caller (the shell
  surface) needs them to register and run scripts.

## Access and history

- Commands are in the `agent_scripts` category. They use the agent policies
  triggers use (`AGENT_VIEW` for reads, `AGENT_MANAGE` for writes); no policy is
  widened for scripts.
- The worker reaches them as the org's internal caller (`user_id: None` on
  gRPC `ExecuteCommand`), which `Command::run` lets through like every other
  internal-caller command.
- Every mutation is recorded in the generic change history as entity kind
  `agent_script`; restore goes through `update_agent_script`
  ([change reasons](../execution/change-reasons-and-manager-context.md)).
