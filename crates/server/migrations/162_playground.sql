ALTER TABLE sessions DROP CONSTRAINT sessions_source_check;
ALTER TABLE sessions ADD CONSTRAINT sessions_source_check CHECK (source IN (
    'chat', 'playground', 'api', 'slack', 'ag_ui', 'fcp', 'schedule',
    'webhook', 'a2a', 'eval', 'subagent', 'unknown'
));
ALTER TABLE sessions ADD COLUMN playground_user_id UUID;
ALTER TABLE sessions ADD CONSTRAINT sessions_playground_user_org_fk
    FOREIGN KEY (org_id, playground_user_id) REFERENCES virtual_users(org_id, id) ON DELETE RESTRICT;
ALTER TABLE sessions ADD CONSTRAINT sessions_playground_user_shape
    CHECK ((source = 'playground') = (playground_user_id IS NOT NULL));
