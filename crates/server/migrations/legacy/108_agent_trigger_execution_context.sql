-- Preserve an explicit execution context on agent triggers.
--
-- Migration 104 created `agent_triggers` and 106 retargeted App schedule
-- channels onto agent triggers, but neither carried the App's execution context
-- (harness, owner principal, resolved user, agent identity, source app). Without
-- it a migrated schedule silently runs under the agent's own identity, which can
-- change harness capabilities, private-memory scope, and identity-backed
-- providers relative to the original App.
--
-- These columns let a trigger carry that context. Native agent triggers leave
-- them NULL and are owned by the agent's own identity; a populated context is
-- used verbatim (see resolve_trigger_execution_context in
-- domains/agent_triggers/commands.rs). 104 and 106 are immutable once merged, so
-- the columns are added here rather than edited into those files.
--
-- NOTE: rows already migrated by 106 have no surviving link back to their source
-- App (106 deletes the app_channels rows), so they cannot be backfilled here and
-- continue to fall back to agent-identity ownership. New context is captured
-- when a trigger is created with it going forward.

ALTER TABLE agent_triggers
    ADD COLUMN execution_harness_id UUID REFERENCES harnesses(id) ON DELETE RESTRICT,
    ADD COLUMN execution_owner_principal_id UUID REFERENCES principals(id) ON DELETE RESTRICT,
    ADD COLUMN execution_resolved_owner_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    ADD COLUMN execution_agent_identity_id UUID REFERENCES agent_identities(id) ON DELETE SET NULL,
    ADD COLUMN execution_app_id UUID REFERENCES apps(id) ON DELETE SET NULL;
