-- Organization tool approval policy: which approval modes (off, auto, normal,
-- protective) people and agents may use, rules for every agent, and the trust
-- notes the auto reviewer reads. NULL means no limits. The shape is validated
-- by the API (`OrgToolApprovalPolicy` in everruns-contracts). See
-- knowledge/execution/tool-approval.md.
ALTER TABLE organization_settings
ADD COLUMN tool_approval_policy JSONB;
