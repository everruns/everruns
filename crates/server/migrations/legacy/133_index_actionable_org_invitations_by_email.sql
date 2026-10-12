CREATE INDEX idx_org_invitations_actionable_email_created_at
    ON org_invitations (email, created_at DESC)
    WHERE accepted_at IS NULL AND revoked_at IS NULL;
