-- Seed a well-known anonymous user for auth=none mode.
-- UUID: 00000000-0000-0000-0000-000000000001
-- This ensures all code paths (org membership, API keys, etc.) work
-- without special-casing a nil/missing user.

INSERT INTO users (id, email, name, roles, email_verified, auth_provider)
VALUES (
    '00000000-0000-0000-0000-000000000001',
    'anonymous@local',
    'Anonymous',
    '["admin"]'::jsonb,
    TRUE,
    'none'
)
ON CONFLICT (id) DO NOTHING;

-- Add anonymous user to default organization
INSERT INTO organization_members (org_id, user_id)
VALUES (1, '00000000-0000-0000-0000-000000000001')
ON CONFLICT (org_id, user_id) DO NOTHING;
