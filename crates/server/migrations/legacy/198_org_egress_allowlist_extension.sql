-- Per-organization extension of the system egress allowlist.
--
-- A platform user grants an org the right to extend the curated allowlist
-- (`egress_allowlist_extension_granted`); the org's admins then maintain the
-- extra host patterns (`egress_allowlist_extension`). The patterns widen the
-- allowlist for that org's runtime egress only while the grant is on; revoking
-- the grant keeps them stored but stops enforcing them. The deny list still
-- wins. See knowledge/operations/system-allowlist.md ("Org extensions").
ALTER TABLE organization_settings
ADD COLUMN egress_allowlist_extension_granted BOOLEAN NOT NULL DEFAULT false,
ADD COLUMN egress_allowlist_extension TEXT[] NOT NULL DEFAULT '{}'
    CHECK (cardinality(egress_allowlist_extension) <= 50);
