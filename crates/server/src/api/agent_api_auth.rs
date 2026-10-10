// Who is calling an `api` channel: the credential checks of the Agent
// Execution API, shared by its routes and by `/runtime-auth`.
//
// Decisions (see knowledge/integrations/agent-execution-api.md):
// - Methods are chosen by credential shape, never by trying them blindly: an
//   `evr_ak_` bearer is an agent key; any other bearer is first checked as a
//   runtime token Everruns minted for this channel, then against the channel's
//   `auth_methods` in order (the customer's identity providers).
// - `End-User` is honoured only from a key holding `end_user`. The binding
//   realm is the key, so two applications cannot collide on "customer 42",
//   and a caller who is already an end user cannot assert anyone else.
// - A runtime token is accepted only by the channel it names, and is never
//   exchanged for a new one, so a leaked token cannot renew itself.
// - An `evr_pat_` bearer is a member of the owning organization, accepted only
//   when the channel opts in (`org_members`). The member runs as their default
//   runtime account, so their own connections apply, exactly as in the
//   console. Console session tokens are not accepted: their JWT shape would
//   be ambiguous with the customer's OIDC tokens.
// THREAT[TM-AGENTKEY-007]: end-user assertion spoofing.

use axum::http::HeaderMap;
use everruns_contracts::typed_id::VirtualUserId;

use super::channel_auth::{
    ChannelAuthError, ChannelAuthVerifier, LegacyChannelAuth, extract_bearer,
};
use super::channel_ingress::{IngressChannel, IngressContext};
use crate::auth::AuthState;
use crate::auth::personal_access_token::PAT_PREFIX;
use crate::domains::agent_channels::api_sessions::{
    ApiAuthError, ApiCaller, EndUser, api_channel_config, end_user_principal, resolve_end_user,
    valid_end_user_id, verify_agent_key,
};
use crate::domains::agent_channels::record::api::{
    AGENT_KEY_PREFIX, AGENT_KEY_PROVIDER, AgentApiChannelConfig, END_USER_HEADER,
    agent_key_public_id,
};
use crate::storage::StorageBackend;
use std::sync::Arc;

/// What a caller's credential resolves against.
pub(crate) struct CallerChecks<'a> {
    pub db: &'a Arc<StorageBackend>,
    pub verifier: &'a ChannelAuthVerifier,
    /// Present when runtime tokens can be checked here.
    pub runtime_auth: Option<&'a AuthState>,
}

/// A verified identity before it becomes an end user: who vouched for it and
/// whom it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedIdentity {
    pub provider: String,
    pub realm: String,
    pub subject: String,
    /// The agent key that asserted it, for `End-User`.
    pub key_id: Option<uuid::Uuid>,
}

/// The caller of one request to `channel`.
pub(crate) async fn resolve_caller(
    checks: &CallerChecks<'_>,
    context: &IngressContext,
    channel: &IngressChannel,
    headers: &HeaderMap,
) -> anyhow::Result<Result<ApiCaller, ApiAuthError>> {
    let config = match api_channel_config(channel) {
        Ok(config) => config,
        Err(error) => return Ok(Err(error)),
    };
    let Some(bearer) = extract_bearer(headers) else {
        return Ok(Err(ApiAuthError::Unauthorized));
    };
    if bearer.starts_with(PAT_PREFIX) {
        if headers.contains_key(END_USER_HEADER) {
            return Ok(Err(ApiAuthError::Forbidden));
        }
        let member = if config.org_members {
            org_member(checks, context, bearer).await?
        } else {
            None
        };
        return Ok(match member {
            Some(user) => Ok(ApiCaller::end_user(None, user, config)),
            None => Err(ApiAuthError::Unauthorized),
        });
    }
    if !bearer.starts_with(AGENT_KEY_PREFIX)
        && let Some(user) = runtime_token_user(checks, context, channel, bearer).await?
    {
        if headers.contains_key(END_USER_HEADER) {
            return Ok(Err(ApiAuthError::Forbidden));
        }
        return Ok(Ok(ApiCaller::end_user(None, user, config)));
    }
    let identity = match verified_identity(checks, context, channel, &config, headers).await? {
        Ok(Some(identity)) => identity,
        Ok(None) => {
            // A key acting as itself.
            let key = verify_agent_key(checks.db, context, channel, bearer).await?;
            return Ok(match key {
                Some(key) => Ok(ApiCaller::key(key.key_id, config)),
                None => Err(ApiAuthError::Unauthorized),
            });
        }
        Err(error) => return Ok(Err(error)),
    };
    let user = resolve_end_user(
        checks.db,
        context.org_id,
        &identity.provider,
        &identity.realm,
        &identity.subject,
    )
    .await?;
    Ok(match user {
        Some(user) => Ok(ApiCaller::end_user(identity.key_id, user, config)),
        None => Err(ApiAuthError::Unauthorized),
    })
}

/// The end-user identity a request proves, without runtime tokens: a key
/// with `End-User`, or a token of one of the channel's identity providers.
/// `Ok(None)` means "an agent key with no `End-User`": the caller is the key.
pub(crate) async fn verified_identity(
    checks: &CallerChecks<'_>,
    context: &IngressContext,
    channel: &IngressChannel,
    config: &AgentApiChannelConfig,
    headers: &HeaderMap,
) -> anyhow::Result<Result<Option<VerifiedIdentity>, ApiAuthError>> {
    let Some(bearer) = extract_bearer(headers) else {
        return Ok(Err(ApiAuthError::Unauthorized));
    };
    let end_user = match headers.get(END_USER_HEADER) {
        None => None,
        Some(value) => match value.to_str().ok().filter(|id| valid_end_user_id(id)) {
            Some(id) => Some(id.to_string()),
            None => return Ok(Err(ApiAuthError::Forbidden)),
        },
    };
    if bearer.starts_with(AGENT_KEY_PREFIX) {
        let Some(id) = end_user else {
            return Ok(Ok(None));
        };
        let Some(key) = verify_agent_key(checks.db, context, channel, bearer).await? else {
            return Ok(Err(ApiAuthError::Unauthorized));
        };
        if !key.may_act_for_end_users {
            return Ok(Err(ApiAuthError::Forbidden));
        }
        return Ok(Ok(Some(VerifiedIdentity {
            provider: AGENT_KEY_PROVIDER.to_string(),
            realm: agent_key_public_id(key.key_id),
            subject: id,
            key_id: Some(key.key_id),
        })));
    }
    if end_user.is_some() {
        // Only an application's key may say whom it acts for.
        return Ok(Err(ApiAuthError::Forbidden));
    }
    for method in &config.auth_methods {
        match checks
            .verifier
            .verify_principal(
                method,
                headers,
                LegacyChannelAuth {
                    shared_secret: None,
                    api_key: None,
                },
            )
            .await
        {
            Ok(Some(principal)) => {
                return Ok(Ok(Some(VerifiedIdentity {
                    provider: principal.provider,
                    realm: principal.identity_realm,
                    subject: principal.subject,
                    key_id: None,
                })));
            }
            Ok(None) | Err(ChannelAuthError::Unauthorized) => {}
            Err(error) => {
                tracing::warn!(
                    channel_id = %channel.public_id,
                    ?error,
                    "api channel identity provider unavailable or misconfigured"
                );
            }
        }
    }
    Ok(Err(ApiAuthError::Unauthorized))
}

/// The end user a runtime token minted for this channel carries, or `None`
/// when `token` is not such a token.
async fn runtime_token_user(
    checks: &CallerChecks<'_>,
    context: &IngressContext,
    channel: &IngressChannel,
    token: &str,
) -> anyhow::Result<Option<EndUser>> {
    let Some(auth) = checks.runtime_auth else {
        return Ok(None);
    };
    let jwt = crate::auth::jwt::JwtService::new(auth.config.jwt.clone());
    if jwt.validate_runtime_token(token).is_err() {
        return Ok(None);
    }
    // An authentic runtime token of another channel, or one whose binding or
    // channel went away, is refused rather than tried as an OIDC token.
    let Ok(account) = crate::auth::runtime::RuntimeAccount::from_token(auth, token).await else {
        return Ok(None);
    };
    if account.org_id != context.org_id
        || account.channel_id.as_deref() != Some(&channel.public_id.to_string())
    {
        return Ok(None);
    }
    runtime_user(checks.db, account.org_id, account.id).await
}

/// The organization member a personal access token belongs to, as the end
/// user they run as here, or `None` for an invalid token or a non-member.
async fn org_member(
    checks: &CallerChecks<'_>,
    context: &IngressContext,
    token: &str,
) -> anyhow::Result<Option<EndUser>> {
    let Some(auth) = checks.runtime_auth else {
        return Ok(None);
    };
    let Ok(user) = auth.backend.validate_personal_access_token(token).await else {
        return Ok(None);
    };
    if !user.is_member_of(context.org_id) {
        return Ok(None);
    }
    let account = checks
        .db
        .default_virtual_user(context.org_id, user.id)
        .await?;
    if account.status != "active" {
        return Ok(None);
    }
    let principal = crate::domains::users::PrincipalService::new(checks.db.clone())
        .ensure_default_virtual_user_principal_for(context.org_id, user.id, &account)
        .await?;
    Ok(Some(EndUser {
        virtual_user_id: account.id,
        principal_id: principal.id,
    }))
}

async fn runtime_user(
    db: &Arc<StorageBackend>,
    org_id: i64,
    id: VirtualUserId,
) -> anyhow::Result<Option<EndUser>> {
    let Some(user) = db.get_virtual_user(org_id, id).await? else {
        return Ok(None);
    };
    end_user_principal(db, org_id, user.id, &user.status).await
}
