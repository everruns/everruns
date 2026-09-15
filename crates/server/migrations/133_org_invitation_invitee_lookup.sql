CREATE INDEX idx_org_invitations_invitee_pending
    ON org_invitations(email)
    WHERE accepted_at IS NULL AND revoked_at IS NULL;
