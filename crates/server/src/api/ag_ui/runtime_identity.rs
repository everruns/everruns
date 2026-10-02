use super::*;

pub(super) async fn authorize_ag_ui_request(
    state: &AgUiState,
    target: AgUiTarget,
    headers: &HeaderMap,
    peer_addr: Option<std::net::SocketAddr>,
) -> Result<AuthorizedAgUiRequest, Response> {
    let (context, channel) = match target {
        AgUiTarget::LegacyApp(app_id) => {
            match crate::api::endpoint_ingress::resolve_legacy_endpoint(
                &state.db,
                state.encryption.as_ref(),
                &app_id,
                EndpointTransport::AgUi,
            )
            .await
            .map_err(internal_error)?
            {
                crate::api::endpoint_ingress::LegacyEndpointMatch::One(endpoint) => *endpoint,
                crate::api::endpoint_ingress::LegacyEndpointMatch::NotFound => {
                    return Err(not_found());
                }
                crate::api::endpoint_ingress::LegacyEndpointMatch::Ambiguous => {
                    return Err(conflict(
                        "Multiple enabled AG-UI channels; use an endpoint-scoped /v1/e/{channel_id}/ag-ui URL",
                    ));
                }
            }
        }
        AgUiTarget::Endpoint(channel_id) => crate::api::endpoint_ingress::resolve_endpoint(
            &state.db,
            state.encryption.as_ref(),
            &channel_id,
        )
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?,
    };

    // THREAT[TM-AUTHZ-005]: Anonymous AG-UI requests must not reach draft or
    // private app configurations.
    // Mitigation: Require a published app, an enabled AG-UI channel, and
    // `anonymous=true` before accepting unauthenticated traffic.
    //
    // THREAT[TM-TENANT-002]: An unauthenticated caller must not be able to tell
    // "app does not exist" apart from "app exists but is not published / has no
    // AG-UI channel / is misconfigured". Every such case collapses to a single
    // generic 404 (matching the FCP channel in `api/fcp.rs`); the real reason is
    // logged server-side only.
    if channel.channel_type != EndpointTransport::AgUi {
        return Err(not_found());
    }
    if let Err(reason) = crate::api::endpoint_ingress::endpoint_liveness(&context, &channel) {
        tracing::debug!(
            app_id = %context.public_id,
            endpoint_id = %channel.public_id,
            reason = reason.as_str(),
            "AG-UI request rejected: endpoint not live"
        );
        return Err(not_found());
    }

    let Some(channel_config) = channel.ag_ui_config() else {
        tracing::error!(app_id = %context.public_id, "AG-UI channel config did not deserialize");
        return Err(not_found());
    };
    let runtime_user = if let Some((account, _)) =
        runtime_endpoint_account(state, &channel.public_id.to_string(), headers).await?
    {
        Some(account.id)
    } else if let Some(auth) = channel.auth.as_ref() {
        let principal = state
            .auth_verifier
            .verify_principal(
                auth,
                headers,
                LegacyEndpointAuth {
                    shared_secret: channel_config.token.as_deref(),
                    api_key: None,
                },
            )
            .await
            .map_err(ag_ui_auth_error_response)?;
        if let Some(principal) = principal {
            Some(
                resolve_ingress_identity(
                    state,
                    context.org_id,
                    "oidc",
                    &principal.issuer,
                    &principal.subject,
                )
                .await?,
            )
        } else {
            None
        }
    } else {
        if !channel_config.anonymous {
            // No auth provider configured and anonymous access disabled: the
            // channel is not reachable. Collapse to a generic 404 rather than a
            // 403 so callers cannot confirm the app exists (TM-TENANT-002).
            tracing::debug!(app_id = %context.public_id, "AG-UI request rejected: anonymous access disabled with no auth provider");
            return Err(not_found());
        }
        if let Some(expected_token) = channel_config.token.as_deref()
            && !expected_token.is_empty()
        {
            let provided_token = extract_ag_ui_token(headers).ok_or_else(unauthorized)?;
            if !constant_time_eq(provided_token.as_bytes(), expected_token.as_bytes()) {
                return Err(unauthorized());
            }
        }
        None
    };

    // THREAT[TM-DOS-010]: Anonymous AG-UI traffic must respect a configurable
    // per-app, per-IP cap in addition to the global API limit. App owners
    // tune `rate_limit_per_minute` based on expected client traffic.
    if let Some(limit) = channel_config.rate_limit_per_minute
        && limit > 0
    {
        let client_ip = extract_client_ip_from_parts(peer_addr, headers);
        if state
            .rate_limiter
            .check(
                &format!("{}:{}", context.public_id, channel.public_id),
                client_ip,
                limit,
            )
            .await
            .is_err()
        {
            return Err(too_many_requests("AG-UI rate limit exceeded for this app"));
        }
    }

    Ok(AuthorizedAgUiRequest {
        channel_id: channel.public_id.to_string(),
        endpoint_internal_id: channel.internal_id,
        context,
        channel_config,
        runtime_user,
    })
}

pub(crate) async fn resolve_ingress_identity(
    state: &AgUiState,
    org: i64,
    provider: &str,
    realm: &str,
    subject: &str,
) -> Result<everruns_provider::typed_id::VirtualUserId, Response> {
    let user = state
        .db
        .resolve_runtime_identity(crate::storage::runtime_identity::VerifiedRuntimeIdentity {
            org_id: org,
            provider: provider.into(),
            realm: realm.into(),
            subject: subject.into(),
            name: "User".into(),
            avatar_url: None,
            management_user_id: None,
        })
        .await
        .map_err(internal_error)?;
    if user.status != "active" {
        return Err(unauthorized());
    }
    Ok(user.id)
}

pub(crate) async fn runtime_endpoint_account(
    state: &AgUiState,
    endpoint: &str,
    headers: &HeaderMap,
) -> Result<
    Option<(
        crate::auth::runtime::RuntimeAccount,
        crate::storage::runtime_identity::VirtualUserBindingRow,
    )>,
    Response,
> {
    let Some(auth) = &state.runtime_auth else {
        return Ok(None);
    };
    let Some(token) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return Ok(None);
    };
    let jwt = crate::auth::jwt::JwtService::new(auth.config.jwt.clone());
    // An authentic runtime token stays in its own audience and exact endpoint.
    if jwt.validate_runtime_token(token).is_err() {
        return Ok(None);
    }
    let account = crate::auth::runtime::RuntimeAccount::from_token(auth, token)
        .await
        .map_err(|_| unauthorized())?;
    if account.endpoint_id.as_deref() != Some(endpoint) {
        return Err(unauthorized());
    }
    let claims = jwt
        .validate_runtime_token(token)
        .map_err(|_| unauthorized())?;
    let binding = state
        .db
        .list_virtual_user_bindings(account.org_id, account.id)
        .await
        .map_err(internal_error)?
        .into_iter()
        .find(|b| b.id == claims.binding_id)
        .ok_or_else(unauthorized)?;
    Ok(Some((account, binding)))
}
