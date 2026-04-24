-- Backfill role for organizations created before role-based memberships existed.
-- Choose a deterministic owner (earliest membership) only when an org has no owner/admin.
WITH orgs_without_privileged_roles AS (
    SELECT om.org_id
    FROM organization_members om
    GROUP BY om.org_id
    HAVING COUNT(*) FILTER (WHERE om.role IN ('owner', 'admin')) = 0
), first_member_per_org AS (
    SELECT DISTINCT ON (om.org_id)
        om.org_id,
        om.user_id
    FROM organization_members om
    JOIN orgs_without_privileged_roles o ON o.org_id = om.org_id
    ORDER BY om.org_id, om.created_at ASC, om.user_id ASC
)
UPDATE organization_members om
SET role = 'owner'
FROM first_member_per_org f
WHERE om.org_id = f.org_id
  AND om.user_id = f.user_id;
