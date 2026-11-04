//! Server-derived provenance of an inbound MCP request. Never an identity
//! assertion about the software vendor: registration names are self-declared.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct McpClientAttribution {
    pub client_id: String,
    pub grant_id: Uuid,
    /// Self-declared registration name, snapshotted at the time of the action.
    pub client_name: String,
}

tokio::task_local! {
    static CLIENT: (Uuid, Option<McpClientAttribution>);
}

/// Only the authenticated MCP adapter may install request provenance. It is
/// deliberately not inherited by spawned work or derived from session state.
pub(crate) fn scope<F: std::future::Future>(
    user: &super::AuthUser,
    future: F,
) -> impl std::future::Future<Output = F::Output> {
    // MCP dispatch composes large command futures. Box before adding the
    // task-local wrapper so ordinary Tokio worker stacks remain sufficient.
    CLIENT.scope((user.id, user.mcp_client.clone()), Box::pin(future))
}

// THREAT[TM-MCP-014]: attribution comes from the validated grant, never clientInfo.
pub(crate) fn for_user(user_id: Option<Uuid>) -> Option<McpClientAttribution> {
    CLIENT
        .try_with(|(id, client)| {
            if user_id == Some(*id) {
                client.clone()
            } else {
                None
            }
        })
        .ok()
        .flatten()
}

pub(crate) fn scope_command<F: std::future::Future>(
    ctx: &crate::domains::common::Ctx,
    future: F,
) -> impl std::future::Future<Output = F::Output> {
    let client = ctx
        .change_intent
        .as_ref()
        .and_then(|i| i.mcp_client.clone())
        .filter(|(user, _)| Some(*user) == ctx.caller.user_id)
        .map(|(_, client)| client)
        .or_else(|| for_user(ctx.caller.user_id));
    CLIENT.scope(
        (ctx.caller.user_id.unwrap_or_default(), client),
        Box::pin(future),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn provenance_is_per_request_per_user_and_not_inherited_by_background_work() {
        let mut user = super::super::AuthUser::anonymous();
        let first = McpClientAttribution {
            client_id: "client_a".into(),
            grant_id: Uuid::now_v7(),
            client_name: "Claimed name".into(),
        };
        user.mcp_client = Some(first.clone());
        // A large command must not inflate the request wrapper's stack frame.
        let large = std::future::ready([0u8; 64 * 1024]);
        assert!(std::mem::size_of_val(&scope(&user, large)) < 1024);
        scope(&user, async {
            assert_eq!(for_user(Some(user.id)), Some(first.clone()));
            assert_eq!(for_user(Some(Uuid::now_v7())), None);
            let user_id = user.id;
            assert_eq!(
                tokio::spawn(async move { for_user(Some(user_id)) })
                    .await
                    .unwrap(),
                None
            );
            let mut second = user.clone();
            second.mcp_client.as_mut().unwrap().client_id = "client_b".into();
            scope(&second, async {
                assert_eq!(for_user(Some(user.id)), second.mcp_client);
            })
            .await;
            assert_eq!(for_user(Some(user.id)), Some(first));
        })
        .await;
        assert_eq!(for_user(Some(user.id)), None);
    }

    #[tokio::test]
    async fn command_context_carries_provenance_only_for_its_original_actor() {
        use crate::domains::common::Ctx;
        use crate::storage::StorageBackend;
        use everruns_core::{Caller, DEFAULT_ORG_ID, organization::OrgRole};
        use std::sync::Arc;

        let mut user = super::super::AuthUser::anonymous();
        user.id = Uuid::now_v7();
        user.mcp_client = Some(McpClientAttribution {
            client_id: "client_a".into(),
            grant_id: Uuid::now_v7(),
            client_name: "A".into(),
        });
        let caller = Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: "org_00000000000000000000000000000001".into(),
            user_id: Some(user.id),
            role: OrgRole::Owner,
            is_platform_user: false,
            is_internal: false,
            read_only: false,
        };
        let ctx = Ctx::minimal_for_test(caller, Arc::new(StorageBackend::test_database()), None);
        let mut captured = scope(&user, async {
            crate::domains::change_history::on_surface(ctx)
        })
        .await;
        assert_eq!(for_user(Some(user.id)), None);
        let large = std::future::ready([0u8; 64 * 1024]);
        assert!(std::mem::size_of_val(&scope_command(&captured, large)) < 1024);
        scope_command(&captured, async {
            assert_eq!(for_user(Some(user.id)), user.mcp_client);
        })
        .await;
        captured.caller.user_id = Some(Uuid::now_v7());
        scope_command(&captured, async {
            assert_eq!(for_user(captured.caller.user_id), None);
            assert_eq!(for_user(Some(user.id)), None);
        })
        .await;
    }
}
