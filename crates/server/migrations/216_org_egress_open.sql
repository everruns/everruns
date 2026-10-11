-- Per-organization open egress grant.
--
-- A platform user (or the hosting layer, for orgs that paid with a card) can
-- grant an org open egress: its runtime requests reach any public host the
-- deny list and SSRF checks allow, reads and writes alike, whatever the
-- deployment's curated policy. Volume is bounded outside this table (an egress
-- proxy, the hosted ledger). See knowledge/operations/system-allowlist.md
-- ("Org grants").
ALTER TABLE organization_settings
ADD COLUMN egress_open BOOLEAN NOT NULL DEFAULT false;
