-- Who answers an org's deployment-owned decision checks: guardrail `jev`
-- checks and the Slack respond-or-not relevance check.
--
-- 'deployment' (the default) keeps the deployment's decisions service, so
-- existing orgs are unchanged. 'organization' sends those checks to the org's
-- decision default (decision_model_defaults) on its own provider account. An
-- org that opted in but has no usable model fails open (guardrails) or stays
-- silent (Slack); it never falls back to the deployment's keys.
ALTER TABLE organization_settings
ADD COLUMN system_decisions TEXT NOT NULL DEFAULT 'deployment'
    CHECK (system_decisions IN ('deployment', 'organization'));
