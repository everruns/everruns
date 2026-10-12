-- Re-key `agent` budget subjects onto the agent's public id (EVE-1136).
--
-- Budget subjects are the identifiers the API exposes, because those are the
-- only identifiers a caller can put in a budget. Every other subject type
-- already obeys this: `session` and `org` carry prefixed public ids, and
-- migration 137 keyed `agent_endpoint` on `agent_endpoints.public_id`.
--
-- The `agent` level did not. The resolver rendered the typed `AgentId` taken
-- off `sessions.agent_id` — an FK to `agents(id)` — which spells the *internal*
-- uuid, while `POST /v1/budgets` stores whatever the caller sent, and the
-- caller only ever has `agents.public_id`. `agents.id` and `agents.public_id`
-- are generated independently (the insert lets `id` default to `uuidv7()` and
-- binds `public_id` separately), so the two never coincide and agent-scoped
-- budgets were stored, listed and displayed but never evaluated.
--
-- The resolver now looks the public id up. Any row already written in the
-- internal spelling would stop being found by that lookup, so re-key it here:
-- a budget that silently stops binding is the one regression this fix must not
-- cause. In practice these rows come from internal callers and fixtures, since
-- the internal spelling was not reachable through the API.
UPDATE budgets AS b
SET subject_id = a.public_id,
    updated_at = NOW()
FROM agents AS a
WHERE b.subject_type = 'agent'
  AND b.org_id = a.org_id
  AND b.subject_id = 'agent_' || replace(a.id::text, '-', '')
  AND b.subject_id IS DISTINCT FROM a.public_id;

-- A row re-keyed onto an agent that already had a budget is intentionally left
-- alongside it rather than merged: `check_budgets_for_session` evaluates every
-- matching budget and keeps the most restrictive result, so two rows on one
-- agent bind as the tighter of the two. Merging them would have to pick a
-- limit, and picking the looser one would raise a ceiling nobody asked to
-- raise.
