// Per-organization egress allowlist extension.
//
// Decision: two authorities, two endpoints. A platform user (`Rule::IsPlatformUser`)
// grants or revokes an org's right to extend the curated system allowlist; the
// org's own admins (`OrgSettingsManage`) maintain the patterns once granted.
// Revoking keeps the stored patterns but stops their enforcement, so a
// re-grant restores them without the tenant re-entering anything. The runtime
// reads the enforced extension through the internal command below (gRPC
// workers) or straight from storage (the server process); either way the
// egress boundary caches it for `ORG_EGRESS_ALLOWLIST_CACHE_TTL`. See
// knowledge/operations/system-allowlist.md ("Org extensions") and
// THREAT[TM-AGENT-036].

use crate::auth::audit;
use crate::domains::audit_logs::record::{AuditEvent, ManagementAction};
use crate::domains::common::*;
use crate::domains::organizations::record::validate_org_public_id;
use crate::storage::org_egress_allowlist::OrgEgressAllowlistRow;
use everruns_contracts::runtime::{
    EgressPolicyMode, MAX_ORG_EGRESS_ALLOWLIST_PATTERNS, SystemEgressPolicy,
    validate_org_egress_patterns,
};
use everruns_core::{Permission, Policy, Rule};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Policy: read an org's egress allowlist extension.
pub const ORG_EGRESS_ALLOWLIST_VIEW: Policy = Policy {
    id: "org.egress_allowlist.view",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsView)],
};

/// Policy: edit an org's extension patterns (the grant is checked in the command).
pub const ORG_EGRESS_ALLOWLIST_MANAGE: Policy = Policy {
    id: "org.egress_allowlist.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsManage)],
};

/// Policy: grant or revoke an org's right to extend the allowlist.
pub const ORG_EGRESS_ALLOWLIST_GRANT: Policy = Policy {
    id: "org.egress_allowlist.grant",
    rules: &[Rule::IsPlatformUser],
};

/// An organization's egress allowlist extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct OrgEgressAllowlistResponse {
    /// Whether a platform administrator allowed this organization to extend
    /// the deployment's outbound allowlist. Patterns are enforced only while
    /// this is true.
    pub granted: bool,
    /// Host patterns this organization adds to the allowlist: `example.com`,
    /// `*.example.com`, or an `https://example.com/path/` prefix.
    pub patterns: Vec<String>,
    /// The deployment's egress policy: `open`, `curated-writes`, or
    /// `curated-all`. Extensions only matter in a curated mode.
    #[schema(example = "curated-writes")]
    pub mode: String,
    /// Most patterns one organization may add.
    pub max_patterns: usize,
    /// Whether the caller may edit the patterns (an organization admin; the
    /// edit also needs `granted`).
    pub can_edit: bool,
    /// Whether the caller may grant or revoke (a platform administrator).
    pub can_grant: bool,
}

impl OrgEgressAllowlistResponse {
    fn from_row(ctx: &Ctx, org: &str, row: OrgEgressAllowlistRow) -> Self {
        let passes = |policy: &Policy| {
            policy
                .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
                .is_ok()
        };
        let mode =
            SystemEgressPolicy::from_env().map_or(EgressPolicyMode::Open, |policy| policy.mode());
        Self {
            granted: row.granted,
            patterns: row.patterns,
            mode: mode.as_str().to_string(),
            max_patterns: MAX_ORG_EGRESS_ALLOWLIST_PATTERNS,
            // Admins edit their active org only (see `target_org_id`).
            can_edit: org == ctx.caller.org_public_id && passes(&ORG_EGRESS_ALLOWLIST_MANAGE),
            can_grant: passes(&ORG_EGRESS_ALLOWLIST_GRANT),
        }
    }
}

/// Body of `PUT /v1/orgs/{org}/egress-allowlist`.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateOrgEgressAllowlistRequest {
    /// The full list of host patterns; replaces the stored list. Blank entries
    /// are ignored and duplicates collapsed.
    pub patterns: Vec<String>,
}

/// Body of `PUT /v1/orgs/{org}/egress-allowlist/grant`.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateOrgEgressAllowlistGrantRequest {
    /// Allow (`true`) or stop (`false`) this organization extending the
    /// allowlist. Revoking keeps its patterns but stops enforcing them.
    pub granted: bool,
}

/// The internal org id the path names, for this caller.
///
/// Org admins act on their active organization only; any other org is
/// NotFound, which keeps org existence private. A platform user may name any
/// org when `platform_reach` is set, as for the operator feature-flag routes.
async fn target_org_id(ctx: &Ctx, org: &str, platform_reach: bool) -> Result<i64, CommandError> {
    if !validate_org_public_id(org) {
        return Err(CommandError::not_found("Organization"));
    }
    if org == ctx.caller.org_public_id {
        return Ok(ctx.org_id());
    }
    if platform_reach && ctx.caller.is_platform_user {
        return ctx
            .db
            .get_organization_by_public_id(org)
            .await?
            .map(|row| row.org_id)
            .ok_or_else(|| CommandError::not_found("Organization"));
    }
    Err(CommandError::not_found("Organization"))
}

fn emit_audit(ctx: &Ctx, org_id: i64, org: &str, setting: &str, detail: serde_json::Value) {
    let event = AuditEvent::management(
        ManagementAction::SettingsUpdated,
        org_id,
        ctx.caller.user_id,
    )
    .target("org", org)
    .detail("setting", setting)
    .detail("change", detail);
    audit::emit_event(ctx.db.clone(), event.build());
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct GetOrgEgressAllowlist {
    /// Organization public ID.
    pub org: String,
}

#[command(
    name = "get_org_egress_allowlist",
    category = "organizations",
    description = "Get the organization's outbound allowlist extension: whether a platform administrator granted it, its host patterns, and the deployment's egress policy mode.",
    method = "GET",
    path = "/v1/orgs/{org}/egress-allowlist",
    tag = "Organizations",
    policy = ORG_EGRESS_ALLOWLIST_VIEW,
    positional = "org",
    http = plain,
    responses((status = 404, description = "Organization not found")),
)]
impl Command for GetOrgEgressAllowlist {
    type Output = OrgEgressAllowlistResponse;

    async fn execute(self, ctx: &Ctx) -> Result<OrgEgressAllowlistResponse, CommandError> {
        let org_id = target_org_id(ctx, &self.org, true).await?;
        let row = ctx.db.org_egress_allowlist(org_id).await?;
        Ok(OrgEgressAllowlistResponse::from_row(ctx, &self.org, row))
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct SetOrgEgressAllowlist {
    /// Organization public ID.
    pub org: String,
    /// The full list of host patterns; replaces the stored list.
    pub patterns: Vec<String>,
}

#[command(
    name = "set_org_egress_allowlist",
    category = "organizations",
    description = "Replace the organization's outbound allowlist extension. Requires an organization admin and a grant from a platform administrator. Patterns must name public hosts: `example.com`, `*.example.com`, or an `https://example.com/path/` prefix.",
    method = "PUT",
    path = "/v1/orgs/{org}/egress-allowlist",
    tag = "Organizations",
    policy = ORG_EGRESS_ALLOWLIST_MANAGE,
    http = plain,
    request_body(UpdateOrgEgressAllowlistRequest),
    responses(
        (status = 400, description = "Invalid pattern or too many patterns"),
        (status = 403, description = "Not an organization admin, or the organization is not granted"),
        (status = 404, description = "Organization not found"),
    ),
)]
impl Command for SetOrgEgressAllowlist {
    type Output = OrgEgressAllowlistResponse;

    async fn execute(self, ctx: &Ctx) -> Result<OrgEgressAllowlistResponse, CommandError> {
        let org_id = target_org_id(ctx, &self.org, false).await?;
        let not_granted = || {
            CommandError::forbidden(
                "A platform administrator must allow this organization to extend the outbound allowlist",
            )
        };
        if !ctx.db.org_egress_allowlist(org_id).await?.granted {
            return Err(not_granted());
        }
        let patterns =
            validate_org_egress_patterns(&self.patterns).map_err(CommandError::bad_request)?;
        // The write re-checks the grant, so a revoke racing this edit wins.
        let row = ctx
            .db
            .set_org_egress_allowlist_patterns(org_id, &patterns)
            .await?
            .ok_or_else(not_granted)?;
        emit_audit(
            ctx,
            org_id,
            &self.org,
            "egress_allowlist",
            serde_json::json!({ "patterns": row.patterns }),
        );
        Ok(OrgEgressAllowlistResponse::from_row(ctx, &self.org, row))
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct SetOrgEgressAllowlistGrant {
    /// Organization public ID.
    pub org: String,
    /// Allow or stop this organization extending the allowlist.
    pub granted: bool,
}

#[command(
    name = "set_org_egress_allowlist_grant",
    category = "organizations",
    description = "Platform administrators only: allow or stop an organization extending the outbound allowlist. Revoking keeps its patterns but stops enforcing them.",
    method = "PUT",
    path = "/v1/orgs/{org}/egress-allowlist/grant",
    tag = "Organizations",
    policy = ORG_EGRESS_ALLOWLIST_GRANT,
    http = plain,
    request_body(UpdateOrgEgressAllowlistGrantRequest),
    responses(
        (status = 403, description = "Platform user access required"),
        (status = 404, description = "Organization not found"),
    ),
)]
impl Command for SetOrgEgressAllowlistGrant {
    type Output = OrgEgressAllowlistResponse;

    async fn execute(self, ctx: &Ctx) -> Result<OrgEgressAllowlistResponse, CommandError> {
        let org_id = target_org_id(ctx, &self.org, true).await?;
        let row = ctx
            .db
            .set_org_egress_allowlist_granted(org_id, self.granted)
            .await?;
        emit_audit(
            ctx,
            org_id,
            &self.org,
            "egress_allowlist_grant",
            serde_json::json!({ "granted": row.granted }),
        );
        Ok(OrgEgressAllowlistResponse::from_row(ctx, &self.org, row))
    }
}

/// The extension the egress boundary enforces for the caller's org.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct EnforcedOrgEgressAllowlist {
    /// Patterns to add to the allowlist; empty when not granted.
    pub patterns: Vec<String>,
}

#[derive(Debug, Default, Deserialize, ToSchema, Serialize)]
pub struct WorkerGetOrgEgressAllowlist {}

#[command(
    name = "worker_get_org_egress_allowlist",
    category = "organizations",
    description = "Internal: the outbound allowlist extension the egress boundary enforces for this organization.",
    method = "GET",
    path = "/internal/orgs/egress-allowlist",
    policy = ORG_EGRESS_ALLOWLIST_VIEW
)]
impl Command for WorkerGetOrgEgressAllowlist {
    type Output = EnforcedOrgEgressAllowlist;

    async fn execute(self, ctx: &Ctx) -> Result<EnforcedOrgEgressAllowlist, CommandError> {
        let row = ctx.db.org_egress_allowlist(ctx.org_id()).await?;
        Ok(EnforcedOrgEgressAllowlist {
            patterns: if row.granted {
                row.patterns
            } else {
                Vec::new()
            },
        })
    }
}

#[cfg(test)]
mod tests;
