-- Org-configurable agent check rules (knowledge/evaluation/agent-checks.md, phase 4).
--
-- One row per (org, rule_id). Two purposes:
--   * builtin_override — toggles/severity-overrides a built-in rule by its id
--     (e.g. "prompt.duplicate_paragraphs"); `config` is null.
--   * custom rule — an org-authored rule, either `declarative` (regex over the
--     resolved prompt) or `nl_rubric` (judged by the utility LLM). `config`
--     holds the rule-type-specific parameters as JSONB.
--
-- Rules are advisory like all agent checks; this table never gates anything.

CREATE TABLE agent_check_rules (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL,
    -- Built-in rule id for overrides, or "custom.<slug>" for custom rules.
    rule_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('builtin_override', 'declarative', 'nl_rubric')),
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    -- Overrides the rule's default severity when set: warning | info | suggestion.
    severity_override TEXT CHECK (severity_override IN ('warning', 'info', 'suggestion')),
    -- Rule-type-specific parameters for custom rules (message, category,
    -- pattern/match_mode for declarative, rubric for nl_rubric). Null for
    -- builtin overrides.
    config JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (org_id, rule_id),
    CONSTRAINT agent_check_rules_org_id_fk
        FOREIGN KEY (org_id) REFERENCES organizations(org_id) ON DELETE CASCADE
);

CREATE INDEX idx_agent_check_rules_org_id ON agent_check_rules(org_id);

CREATE TRIGGER update_agent_check_rules_updated_at
    BEFORE UPDATE ON agent_check_rules
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
