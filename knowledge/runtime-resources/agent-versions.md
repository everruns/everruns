---
type: Specification
title: "Agent Versions"
description: "Immutable Agent configuration snapshots."
tags:
  - everruns
  - runtime-resources
---
# Agent Versions

## Abstract

Agent versions are immutable snapshots of an Agent configuration. They support audit history, rollback, forks, and per-exposure deployment policies without changing the editable Agent draft model.

The pilot is intentionally Agent-specific (`agent_versions`) instead of a generic entity-version table. The model should stay narrow until the product semantics for other versioned entities are proven.

## Requirements

### Data Model

- `agent_versions` stores immutable snapshots for one Agent.
- Each version has both a sequential `version_number` and a semantic `version` string.
- The first saved version is `0.1.0`. `patch`, `minor`, and `major` change kinds bump semantic versions predictably; other change kinds keep the next patch-level sequence unless the server chooses a more specific bump.
- `authored_config` captures the user-authored Agent fields.
- `resolved_config` captures the runtime-effective configuration after capability and prompt resolution. This is the source for version diffs and deterministic runtime binding.
- `config_hash` is computed from authored config for quick equality checks.
- `parent_version_id` links normal history. `source_version_id` records rollback/fork provenance.
- Agent draft rows keep `default_version_id`, fork lineage (`forked_from_agent_id`, `forked_from_version_id`), and `root_agent_id`.
- User-published versions use semantic versions and `is_published = true`. Automatic draft snapshots use `is_published = false`, `change_kind = auto`, and internal labels such as `draft.12`; they do not become defaults or pin targets.

### Runtime Binding

- Sessions capture `agent_version_id` when created if the Agent or the exposure that started them resolves to a version.
- Worker turn loading uses the captured version snapshot instead of the current Agent draft.
- Every exposure carries its own version policy: each endpoint (`agent_endpoints`) and each trigger
  (`agent_triggers.agent_version_*`). A staging endpoint on `latest` and a production
  endpoint `pinned` on the same Agent is the case this exists for.
  - `default`: use the Agent's `default_version_id`. A trigger row with no stored policy means this.
  - `latest`: use the newest saved version for the Agent.
  - `pinned`: use the exposure's `agent_version_id`, regardless of later draft edits or default changes.
- Every ingress path hands the exposure's policy to session creation; the pin is honoured the same
  way for endpoint transports, native triggers, and triggers migrated from App channels.
- Sessions and events must preserve version metadata so traces can be tied back to the exact configuration that ran.

### Product Behavior

- Agent detail exposes a version history tab behind `FEATURE_AGENT_VERSIONS`.
- Users can save a version, set default, compare two versions, roll back a draft, and fork a version into a new Agent.
- Each Agent update records an automatic draft snapshot so normal saves retain rollback history without requiring the user to publish a semantic version.
- The version policy is set per exposure. Endpoint and trigger create/update accept
  `agent_version_policy` and `agent_version_id` and return them on every read, so an org can
  see which exposures are pinned. The UI sets it from the endpoint editor and the trigger
  editor, and lists show a pin badge.
- A pin must name a saved (published) version of the exposure's own Agent in the caller's org;
  `default` and `latest` carry no version, and switching to them clears the pin. Moving off
  `default` needs the `agent_versions` flag, but unpinning is always allowed so a pin carried
  over from the App era can be seen and removed even with the flag off. Validation lives in
  [`version_policy.rs`](../../crates/server/src/domains/agents/version_policy.rs).
- Pins set through the retired App UI were copied onto endpoints and triggers by migrations
  135 and 142; they keep working and are visible through the same fields.
- Rollbacks create a new rollback version by default so history remains append-only.

### Feature Flag

Agent versions follow the rollout-grade policy for `agent_versions`;
`FEATURE_AGENT_VERSIONS` overrides the catalog default. See
[Feature Flags](../security/feature-flags.md).

When disabled:
- Version UI is hidden.
- Version API routes return not found.
- Existing Agent/App/Session fields remain backward-compatible but callers should treat them as inactive.

## Non-Goals

- A/B testing traffic allocation is out of scope.
- Generic `entity_versions` infrastructure is out of scope.
- Editing old versions in place is out of scope; versions are immutable.

## References

- Core types: `crates/platform/src/agent.rs`, `crates/platform/src/app.rs`, `crates/core/src/session.rs`
- Storage models: `crates/server/src/storage/models.rs`
- API commands: `crates/server/src/domains/agents/commands.rs`
- Exposure policy validation: `crates/server/src/domains/agents/version_policy.rs`
- Session-time resolution: `crates/server/src/domains/sessions/service/create.rs`
- Migration: `crates/server/migrations/037_agent_versions.sql`
