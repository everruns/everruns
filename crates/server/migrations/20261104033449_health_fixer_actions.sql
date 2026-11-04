-- Health fixers (knowledge/operations/health-issues.md, "Fixers") record each
-- run as a finding action, so an issue shows who repaired it and when. Slack
-- fixes are recorded with the channel as their target; they cannot be undone.
ALTER TABLE finding_actions DROP CONSTRAINT finding_actions_kind_check;
ALTER TABLE finding_actions ADD CONSTRAINT finding_actions_kind_check
    CHECK (kind IN ('pause_schedule', 'slack.add_permissions', 'slack.reconnect'));
