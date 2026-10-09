// PACT Delegated `message:send` (PACT 1.0 §5.5, §5.6): a personal agent sends
// the user's delegation token with its own JWT, the turn runs as that company
// user, a tool that needs a scope the user has not granted turns the answer
// into a step-up request, and every reply under a token carries a signed
// receipt of what the agent did for the user.
//
// Design Decisions:
// - The turn runs as an Everruns end user bound to (endpoint, company `sub`),
//   and the token becomes that session's grant for the catalog MCP server the
//   config names (`mcp_server`). The server is attached to the agent as a
//   preset with `actsAs: user`, so the company's API sees the user's own
//   token and enforces its scopes itself. A turn without a token clears the
//   grant, so a later identity-only turn cannot reuse it.
// - Scope checks read the turn's tool events after it ends: a call to a tool
//   a scope lists, without that scope granted, makes the answer a step-up
//   (`TASK_STATE_AUTH_REQUIRED` plus a fresh sign-in link for the missing
//   scopes). The company API refusing the call is what keeps the action from
//   happening; the step-up tells the personal agent how to get permission.
//   A tool several scopes list needs all of them.
// - A context that ran under one company user refuses a token for another
//   (§5.5): the session carries an account tag next to the caller tag.
// - Receipts list the scoped tools that succeeded, each with a SHA-256 of the
//   arguments it ran with, signed with the endpoint's key (`pact_keys.rs`).
// See `knowledge/integrations/a2a-channel.md`.

use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use everruns_contracts::typed_id::{PrincipalId, SessionId, VirtualUserId};
use everruns_core::events::{TOOL_COMPLETED, TOOL_STARTED, ToolCompletedData, ToolStartedData};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::ChannelA2aState;
use super::pact_keys::ProviderKey;
use super::pact_oauth::{ACCESS_TOKEN_TYPE, DelegationClaims, Urls, parse_scope};
use crate::domains::agent_channels::record::pact_delegation::PactDelegationConfig;
use crate::storage::EventRow;
use crate::storage::{UpdateSession, UpsertMcpOAuthSessionCredentials};

/// §5.5: the header a personal agent sends the delegation token in.
pub(super) const DELEGATION_HEADER: &str = "x-a2a-user-delegation";
/// Header `typ` of a receipt JWS (§5.6).
const RECEIPT_TYPE: &str = "pact-receipt+jws";
/// Prefix of the session tag binding a context to one company user.
const ACCOUNT_TAG_PREFIX: &str = "a2a_pact_account:";
/// Virtual users are keyed by (provider, realm, subject).
const IDENTITY_PROVIDER: &str = "pact";

/// A verified delegation token.
pub(super) struct Delegation {
    pub token: String,
    pub sub: String,
    pub client_id: String,
    pub grant_id: String,
    pub scopes: Vec<String>,
}

/// §5.5: a rejected delegation token is a `401` with `error="invalid_token"`
/// and no A2A body, so the personal agent knows to sign the user in again.
pub(super) fn invalid_token() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer realm=\"a2a\", error=\"invalid_token\""),
        )],
    )
        .into_response()
}

/// The request's delegation token, verified. `Ok(None)` when none was sent.
/// THREAT[TM-A2A-018]: the token must be this endpoint's (`typ`, signature,
/// `iss`, `aud`), presented by the personal agent it was issued to, and its
/// grant still live and for the same company user.
pub(super) async fn verify(
    state: &ChannelA2aState,
    channel_internal_id: Uuid,
    interface_url: &str,
    pa_issuer: &str,
    headers: &HeaderMap,
) -> Result<Option<Delegation>, Response> {
    let Some(value) = headers.get(DELEGATION_HEADER) else {
        return Ok(None);
    };
    let token = value
        .to_str()
        .ok()
        .and_then(|value| {
            value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
        })
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(invalid_token)?;
    let key = ProviderKey::for_channel(&state.db, state.encryption.as_ref(), channel_internal_id)
        .await
        .map_err(|err| {
            tracing::error!(error = %err, "PACT signing key unavailable");
            super::http_json::a2a_error(-32603, "Internal error")
        })?;
    let urls = Urls::new(interface_url);
    let claims: DelegationClaims = key
        .verify(ACCESS_TOKEN_TYPE, token, Some(interface_url))
        .ok_or_else(invalid_token)?;
    if claims.iss != urls.issuer || claims.client_id != pa_issuer {
        return Err(invalid_token());
    }
    let grant = state
        .db
        .pact_grant(channel_internal_id, &claims.grant_id)
        .await
        .map_err(|err| {
            tracing::error!(error = %err, "PACT grant lookup failed");
            super::http_json::a2a_error(-32603, "Internal error")
        })?
        .filter(|grant| {
            grant.is_live(chrono::Utc::now())
                && grant.client_id == claims.client_id
                && grant.brand_user_id == claims.sub
        })
        .ok_or_else(invalid_token)?;
    let granted = parse_scope(&grant.scope);
    let scopes = parse_scope(&claims.scope)
        .into_iter()
        .filter(|scope| granted.contains(scope))
        .collect();
    Ok(Some(Delegation {
        token: token.to_string(),
        sub: claims.sub,
        client_id: claims.client_id,
        grant_id: claims.grant_id,
        scopes,
    }))
}

/// The session tag binding a context to one company user of one endpoint.
pub(super) fn account_tag(channel_internal_id: Uuid, sub: &str) -> String {
    let digest = Sha256::new()
        .chain_update(channel_internal_id.as_bytes())
        .chain_update([0u8])
        .chain_update(sub.as_bytes())
        .finalize();
    format!("{ACCOUNT_TAG_PREFIX}{}", &hex::encode(digest)[..32])
}

/// Whether a session with `tags` may run under `delegation`: untagged
/// sessions take the first company user that arrives, tagged ones only that
/// user.
pub(super) fn context_admits(tags: &[String], channel_internal_id: Uuid, sub: &str) -> bool {
    let own = account_tag(channel_internal_id, sub);
    tags.iter()
        .filter(|tag| tag.starts_with(ACCOUNT_TAG_PREFIX))
        .all(|tag| tag == &own)
}

/// The end user a delegated turn runs as, created on first use.
pub(super) async fn runtime_subject(
    state: &ChannelA2aState,
    org_id: i64,
    channel_internal_id: Uuid,
    sub: &str,
) -> anyhow::Result<(PrincipalId, VirtualUserId)> {
    let user = state
        .db
        .resolve_runtime_identity(crate::storage::runtime_identity::VerifiedRuntimeIdentity {
            org_id,
            provider: IDENTITY_PROVIDER.into(),
            realm: format!("{IDENTITY_PROVIDER}:{channel_internal_id}"),
            subject: sub.to_string(),
            name: "PACT delegated user".into(),
            avatar_url: None,
            management_user_id: None,
        })
        .await?;
    let principals = crate::domains::users::PrincipalService::new(state.db.clone());
    let parent = principals
        .ensure_system_principal(org_id, "external-users")
        .await?;
    let principal = principals
        .ensure_virtual_user_principal(org_id, user.id, parent.id)
        .await?;
    Ok((principal.id, user.id))
}

/// Run before the turn is dispatched: tag the session with the company user
/// and hand the token to the company's MCP server, or clear that grant when
/// the turn has no token.
pub(super) async fn prepare_session(
    state: &ChannelA2aState,
    org_id: i64,
    session_id: SessionId,
    config: &PactDelegationConfig,
    user: Option<(&str, VirtualUserId, Uuid, &str)>,
) -> anyhow::Result<()> {
    if let Some((_, _, channel_internal_id, sub)) = user {
        let tag = account_tag(channel_internal_id, sub);
        if let Some(session) = state.db.get_session(org_id, session_id).await?
            && !session.tags.contains(&tag)
        {
            let mut tags = session.tags.clone();
            tags.push(tag);
            state
                .db
                .update_session(
                    org_id,
                    session_id,
                    UpdateSession {
                        tags: Some(tags),
                        ..Default::default()
                    },
                )
                .await?;
        }
    }
    let Some(server_name) = config.mcp_server.as_deref() else {
        return Ok(());
    };
    let Some(server) = state.db.get_mcp_server_by_name(org_id, server_name).await? else {
        tracing::warn!("PACT delegation names an MCP server the org does not have");
        return Ok(());
    };
    let server_id = server.id.uuid();
    match user {
        Some((token, virtual_user_id, _, _)) => {
            let encryption = state
                .encryption
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("PACT delegation needs secrets encryption"))?;
            // No expiry is stored: the grant is rewritten on every delegated
            // turn, and the company API refuses the token once it expires.
            state
                .db
                .upsert_mcp_oauth_session_credentials(UpsertMcpOAuthSessionCredentials {
                    virtual_user_id: Some(virtual_user_id),
                    session_id,
                    server_id,
                    access_token_encrypted: encryption.encrypt_string(token)?,
                    refresh_token_encrypted: None,
                    expires_at_encrypted: None,
                })
                .await?;
        }
        None => {
            for field in ["access_token", "refresh_token", "expires_at"] {
                let name = everruns_core::mcp_oauth_session_secret_name(server_id, field);
                state
                    .db
                    .delete_session_secret(session_id.uuid(), &name)
                    .await?;
            }
        }
    }
    Ok(())
}

/// One tool call of a turn.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ToolRun {
    pub name: String,
    pub success: bool,
    pub arguments: Value,
}

/// Pure: the tool calls among a turn's events (ascending), in call order.
pub(super) fn tool_runs(events: &[EventRow]) -> Vec<ToolRun> {
    let mut arguments = std::collections::HashMap::new();
    let mut runs = Vec::new();
    for event in events {
        match event.event_type.as_str() {
            TOOL_STARTED => {
                if let Ok(data) = serde_json::from_value::<ToolStartedData>(event.data.clone()) {
                    arguments.insert(
                        data.tool_call.id.clone(),
                        data.tool_call.execution_arguments(),
                    );
                }
            }
            TOOL_COMPLETED => {
                if let Ok(data) = serde_json::from_value::<ToolCompletedData>(event.data.clone()) {
                    let started = arguments.remove(&data.tool_call_id);
                    runs.push(ToolRun {
                        arguments: data.executed_arguments.or(started).unwrap_or(Value::Null),
                        name: data.tool_name,
                        success: data.success,
                    });
                }
            }
            _ => {}
        }
    }
    runs
}

/// Pure: scopes the turn's tool calls needed that `granted` lacks, in the
/// order the config lists them.
pub(super) fn missing_scopes(
    config: &PactDelegationConfig,
    granted: &[String],
    runs: &[ToolRun],
) -> Vec<String> {
    config
        .scopes
        .iter()
        .filter(|scope| !granted.contains(&scope.id))
        .filter(|scope| runs.iter().any(|run| scope.tools.contains(&run.name)))
        .map(|scope| scope.id.clone())
        .collect()
}

/// Receipt claims (§5.6).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReceiptClaims {
    grant_id: String,
    user: String,
    pa: String,
    brand: String,
    scopes_used: Vec<String>,
    actions: Vec<Value>,
    ts: String,
}

/// Pure: what the agent did for the user, from the turn's tool calls. Only
/// scoped tools that succeeded count.
pub(super) fn receipt_claims(
    config: &PactDelegationConfig,
    delegation: &Delegation,
    interface_url: &str,
    runs: &[ToolRun],
) -> ReceiptClaims {
    let done: Vec<&ToolRun> = runs
        .iter()
        .filter(|run| run.success && config.scopes_for_tool(&run.name).next().is_some())
        .collect();
    let scopes_used = config
        .scopes
        .iter()
        .filter(|scope| done.iter().any(|run| scope.tools.contains(&run.name)))
        .map(|scope| scope.id.clone())
        .collect();
    let actions = done
        .iter()
        .map(|run| {
            let hash = Sha256::digest(run.arguments.to_string().as_bytes());
            json!({
                "tool": run.name,
                "argsHash": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash),
            })
        })
        .collect();
    ReceiptClaims {
        grant_id: delegation.grant_id.clone(),
        user: delegation.sub.clone(),
        pa: delegation.client_id.clone(),
        brand: interface_url.to_string(),
        scopes_used,
        actions,
        ts: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    }
}

/// The signed receipt: `{jws, claims}`, the JWS payload being the claims.
pub(super) async fn receipt(
    state: &ChannelA2aState,
    channel_internal_id: Uuid,
    claims: &ReceiptClaims,
) -> anyhow::Result<Value> {
    let key =
        ProviderKey::for_channel(&state.db, state.encryption.as_ref(), channel_internal_id).await?;
    Ok(json!({ "jws": key.sign(RECEIPT_TYPE, claims)?, "claims": claims }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> PactDelegationConfig {
        serde_json::from_value(json!({
            "login_url": "https://brand.example/login",
            "login_issuer": "https://brand.example",
            "login_jwks_uri": "https://brand.example/jwks",
            "scopes": [
                { "id": "trips:read", "description": "See trips", "tools": ["list_trips", "change_trip"] },
                { "id": "trips:write", "description": "Change trips", "tools": ["change_trip"] },
                { "id": "profile:read", "description": "See profile", "tools": [] },
            ],
        }))
        .unwrap()
    }

    fn run(name: &str, success: bool) -> ToolRun {
        ToolRun {
            name: name.into(),
            success,
            arguments: json!({ "id": 1 }),
        }
    }

    fn event(event_type: &str, data: Value) -> EventRow {
        EventRow {
            id: everruns_contracts::typed_id::EventId::from_uuid(Uuid::now_v7()),
            session_id: SessionId::from_uuid(Uuid::now_v7()),
            sequence: 0,
            event_type: event_type.to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data,
            metadata: None,
            tags: None,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn a_tool_needs_every_scope_that_lists_it() {
        let config = config();
        let runs = [run("change_trip", false)];
        assert_eq!(
            missing_scopes(&config, &[], &runs),
            ["trips:read", "trips:write"]
        );
        assert_eq!(
            missing_scopes(&config, &["trips:read".into()], &runs),
            ["trips:write"]
        );
        assert!(missing_scopes(&config, &[], &[run("search", true)]).is_empty());
    }

    #[test]
    fn receipts_list_only_scoped_tools_that_succeeded() {
        let config = config();
        let delegation = Delegation {
            token: "t".into(),
            sub: "u1".into(),
            client_id: "https://pa.example".into(),
            grant_id: "g1".into(),
            scopes: vec!["trips:read".into()],
        };
        let claims = receipt_claims(
            &config,
            &delegation,
            "https://x.example/v1/a2a/c",
            &[
                run("list_trips", true),
                run("search", true),
                run("change_trip", false),
            ],
        );
        let value = serde_json::to_value(&claims).unwrap();
        assert_eq!(value["scopesUsed"], json!(["trips:read"]));
        assert_eq!(value["actions"].as_array().unwrap().len(), 1);
        assert_eq!(value["actions"][0]["tool"], "list_trips");
        assert_eq!(value["grantId"], "g1");
        assert_eq!(value["brand"], "https://x.example/v1/a2a/c");
    }

    #[test]
    fn tool_runs_pair_arguments_with_results() {
        let events = [
            event(
                TOOL_STARTED,
                json!({ "tool_call": { "id": "c1", "name": "list_trips", "arguments": { "a": 1 } } }),
            ),
            event(
                TOOL_COMPLETED,
                json!({ "tool_call_id": "c1", "tool_name": "list_trips", "success": true, "status": "success" }),
            ),
        ];
        assert_eq!(
            tool_runs(&events),
            [ToolRun {
                name: "list_trips".into(),
                success: true,
                arguments: json!({ "a": 1 }),
            }]
        );
    }

    #[test]
    fn a_context_admits_only_its_first_account() {
        let channel = Uuid::now_v7();
        let tags = vec!["a2a_caller:x".to_string(), account_tag(channel, "jane")];
        assert!(context_admits(&tags, channel, "jane"));
        assert!(!context_admits(&tags, channel, "joe"));
        assert!(context_admits(&tags[..1], channel, "joe"));
        assert_ne!(
            account_tag(channel, "jane"),
            account_tag(Uuid::now_v7(), "jane")
        );
    }
}
