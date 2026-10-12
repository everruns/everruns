-- Durable onboarding-completion marker for organizations.
--
-- A newly created organization is considered "not yet onboarded" until its
-- creator either finishes or explicitly skips the Configure step of the setup
-- wizard. The zero-org/resume redirect uses a NULL marker on the current org to
-- send the user back to /orgs/{id}/setup on re-entry, so an interrupted
-- onboarding resumes instead of dropping the user into a half-provisioned app.
--
-- CRITICAL BACKFILL: every organization that already exists MUST be treated as
-- already-onboarded. Without the backfill below, all pre-existing orgs would
-- have a NULL marker and their users would be trapped at /setup after this
-- migration deploys. Backfilling to `created_at` marks all current orgs as
-- complete; only orgs created AFTER this migration participate in the resume
-- flow. (Seeded/default and externally-synced orgs are created already-complete
-- by the storage layer, so they are never affected either.)

ALTER TABLE organizations ADD COLUMN onboarding_completed_at TIMESTAMPTZ;

UPDATE organizations
SET onboarding_completed_at = created_at
WHERE onboarding_completed_at IS NULL;
