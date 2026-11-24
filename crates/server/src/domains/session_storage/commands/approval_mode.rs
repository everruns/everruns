//! A person's tool approval mode switch for one session.
//!
//! Decision: the switch is a reserved session key (`tool_approval/mode`) the
//! gate reads on every call, so a change applies to the next tool call without
//! restarting the turn. The model-facing kv tool cannot read or write it
//! (THREAT[TM-TOOL-008]). The org's allowed modes are checked here and again
//! by the gate, which also keeps a person from switching the gate fully off
//! when the agent's manager turned it on (`off` runs as `auto` then).
//! See knowledge/execution/tool-approval.md.

use super::q;
use crate::domains::common::*;
use crate::domains::session_storage::plane::PlaneStorage;
use everruns_contracts::tool_approval_policy::{APPROVAL_MODE_NAMES, SESSION_APPROVAL_MODE_KEY};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The session's approval mode switch and the modes the organization allows.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SessionApprovalMode {
    /// The mode a person picked for this session, or `null` when the session
    /// follows the owner's default and the agent's own mode.
    pub mode: Option<String>,
    /// Modes the organization allows (`off`, `auto`, `normal`, `protective`).
    pub allowed_modes: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct GetSessionApprovalMode {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

#[command(
    name = "get_session_approval_mode",
    category = "sessions",
    description = "Get a session's tool approval mode.",
    method = "GET",
    path = "/v1/platform/sessions/{session_id}/approval-mode",
    policy = crate::domains::sessions::SESSION_VIEW,
    cli = CliRoute::new(&["sessions", "approval-mode"], "get").with_examples(&[CliExample::new("See which approval mode a session runs in", "everruns sessions approval-mode get --session-id session_01h9",)]),
    http = plain,
)]
impl Command for GetSessionApprovalMode {
    type Output = SessionApprovalMode;

    async fn execute(self, ctx: &Ctx) -> Result<SessionApprovalMode, CommandError> {
        let session_id = q::parse_owned_session_id(&self.session_id)?;
        q::verify_session_ownership(&ctx.db, ctx.org_id(), session_id).await?;
        current(ctx, session_id).await
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct SetSessionApprovalMode {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// `off`, `auto`, `normal` or `protective`; `null` clears the switch so
    /// the session follows the owner's default and the agent's mode again.
    #[serde(default)]
    pub mode: Option<String>,
}

#[command(
    name = "set_session_approval_mode",
    category = "sessions",
    description = "Set or clear a session's tool approval mode.",
    method = "PUT",
    path = "/v1/platform/sessions/{session_id}/approval-mode",
    policy = crate::domains::sessions::SESSION_MANAGE,
    cli = CliRoute::new(&["sessions", "approval-mode"], "set").with_examples(&[CliExample::new("Let a reviewer model answer routine approvals in a session", "everruns sessions approval-mode set --session-id session_01h9 --mode auto --reason 'Long refactor, review routine calls automatically'",)]),
    http = plain,
)]
impl Command for SetSessionApprovalMode {
    type Output = SessionApprovalMode;

    async fn execute(self, ctx: &Ctx) -> Result<SessionApprovalMode, CommandError> {
        let session_id = q::parse_owned_session_id(&self.session_id)?;
        q::verify_session_ownership(&ctx.db, ctx.org_id(), session_id).await?;
        let plane = PlaneStorage::of(ctx, session_id).await?;
        match self.mode.as_deref().map(str::trim) {
            None | Some("") => match &plane {
                Some(plane) => {
                    plane.delete_value(SESSION_APPROVAL_MODE_KEY).await?;
                }
                None => {
                    ctx.db
                        .delete_session_key_value(session_id.uuid(), SESSION_APPROVAL_MODE_KEY)
                        .await?;
                }
            },
            Some(mode) => {
                if !APPROVAL_MODE_NAMES.contains(&mode) {
                    return Err(CommandError::bad_request(
                        "mode must be off, auto, normal or protective",
                    ));
                }
                let policy = ctx.db.org_tool_approval_policy(ctx.org_id()).await?;
                if !policy.allows(mode) {
                    return Err(CommandError::bad_request(format!(
                        "your organization does not allow the '{mode}' approval mode"
                    )));
                }
                match &plane {
                    Some(plane) => plane.set_value(SESSION_APPROVAL_MODE_KEY, mode).await?,
                    None => {
                        ctx.db
                            .upsert_session_key_value(crate::storage::UpsertSessionKeyValue {
                                session_id,
                                key: SESSION_APPROVAL_MODE_KEY.to_string(),
                                value: mode.to_string(),
                            })
                            .await?;
                    }
                }
            }
        }
        current(ctx, session_id).await
    }
}

async fn current(
    ctx: &Ctx,
    session_id: crate::kernel_imports::contracts::typed_id::SessionId,
) -> Result<SessionApprovalMode, CommandError> {
    let mode = match PlaneStorage::of(ctx, session_id).await? {
        Some(plane) => plane.value(SESSION_APPROVAL_MODE_KEY).await?,
        None => ctx
            .db
            .get_session_key_value(session_id.uuid(), SESSION_APPROVAL_MODE_KEY)
            .await?
            .map(|row| row.value),
    };
    let policy = ctx.db.org_tool_approval_policy(ctx.org_id()).await?;
    let allowed_modes = APPROVAL_MODE_NAMES
        .iter()
        .filter(|m| policy.allows(m))
        .map(|m| m.to_string())
        .collect();
    Ok(SessionApprovalMode {
        mode,
        allowed_modes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{CreateSessionRow, StorageBackend};
    use everruns_contracts::tool_approval_policy::OrgToolApprovalPolicy;
    use everruns_contracts::typed_id::PrincipalId;
    use everruns_core::{Caller, DEFAULT_ORG_ID};
    use serde_json::{Value, json};
    use std::sync::Arc;

    async fn run(ctx: &Ctx, name: &str, params: Value) -> Result<Value, CommandError> {
        crate::domains::common::dispatch(name, params, ctx)
            .await
            .map(|json| serde_json::from_str(&json).unwrap())
    }

    #[tokio::test]
    async fn switch_round_trips_and_respects_the_org_policy() {
        let db = Arc::new(StorageBackend::test_database());
        let session_id = db
            .create_session(CreateSessionRow {
                org_id: DEFAULT_ORG_ID,
                owner_principal_id: PrincipalId::from_seed(1),
                ..Default::default()
            })
            .await
            .unwrap()
            .id;
        let ctx = Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db.clone(), None);
        let sid = session_id.to_string();

        let fresh = run(
            &ctx,
            "get_session_approval_mode",
            json!({ "session_id": sid }),
        )
        .await
        .unwrap();
        assert_eq!(fresh["mode"], Value::Null);
        assert_eq!(fresh["allowed_modes"].as_array().unwrap().len(), 4);

        let set = run(
            &ctx,
            "set_session_approval_mode",
            json!({ "session_id": sid, "mode": "auto" }),
        )
        .await
        .unwrap();
        assert_eq!(set["mode"], "auto");
        let stored = db
            .get_session_key_value(session_id.uuid(), SESSION_APPROVAL_MODE_KEY)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.value, "auto");

        let bad = run(
            &ctx,
            "set_session_approval_mode",
            json!({ "session_id": sid, "mode": "yolo" }),
        )
        .await
        .expect_err("unknown mode");
        assert!(
            matches!(bad.kind, CommandErrorKind::BadRequest(_)),
            "{bad:?}"
        );

        db.set_org_tool_approval_policy(
            DEFAULT_ORG_ID,
            &OrgToolApprovalPolicy {
                allowed_modes: vec!["normal".into(), "protective".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let refused = run(
            &ctx,
            "set_session_approval_mode",
            json!({ "session_id": sid, "mode": "off" }),
        )
        .await
        .expect_err("org disallows off");
        assert!(
            matches!(refused.kind, CommandErrorKind::BadRequest(_)),
            "{refused:?}"
        );

        let cleared = run(
            &ctx,
            "set_session_approval_mode",
            json!({ "session_id": sid, "mode": null }),
        )
        .await
        .unwrap();
        assert_eq!(cleared["mode"], Value::Null);
        assert_eq!(cleared["allowed_modes"], json!(["normal", "protective"]));
    }
}
