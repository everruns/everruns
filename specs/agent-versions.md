# Agent Versions

## Abstract

Agent versions are immutable snapshots of an Agent configuration. They support audit history, rollback, forks, and App deployment policies without changing the editable Agent draft model.

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

### Runtime Binding

- Sessions capture `agent_version_id` when created if the Agent or App resolves to a version.
- Worker turn loading uses the captured version snapshot instead of the current Agent draft.
- Apps support three version policies:
  - `default`: use the Agent's `default_version_id`.
  - `latest`: use the newest saved version for the Agent.
  - `pinned`: use `agent_version_id` on the App.
- Sessions and events must preserve version metadata so traces can be tied back to the exact configuration that ran.

### Product Behavior

- Agent detail exposes a version history tab behind `FEATURE_AGENT_VERSIONS`.
- Users can save a version, set default, compare two versions, roll back a draft, and fork a version into a new Agent.
- App configuration exposes the Agent version policy behind the same flag.
- Rollbacks create a new rollback version by default so history remains append-only.

### Feature Flag

The pilot is gated by API-visible flag `agent_versions`, resolved from `FEATURE_AGENT_VERSIONS` and auto-enabled in dev grade.

When disabled:
- Version UI is hidden.
- Version API routes return not found.
- Existing Agent/App/Session fields remain backward-compatible but callers should treat them as inactive.

## Non-Goals

- A/B testing traffic allocation is out of scope.
- Generic `entity_versions` infrastructure is out of scope.
- Editing old versions in place is out of scope; versions are immutable.

## References

- Core types: `crates/core/src/agent.rs`, `crates/core/src/app.rs`, `crates/core/src/session.rs`
- Storage models: `crates/server/src/storage/models.rs`
- API commands: `crates/server/src/domains/agents/commands.rs`
- Migration: `crates/server/migrations/037_agent_versions.sql`
