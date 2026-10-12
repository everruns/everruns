//! Browser-facing OAuth connect failures.
//!
//! Decision: a browser that started a connect from a console page goes back to
//! that page with `connect_error=<code>&provider=<provider>` instead of landing
//! on a raw status page (a bare "Bad gateway" for a blocked host). The code is
//! one of a fixed set; the detail (host, upstream status, provider message)
//! stays in the server log. The target is the same validated, same-origin
//! `return_to` the success redirect uses, so this adds no open redirect: an
//! absent or unsafe `return_to` keeps the plain error response.

use axum::http::StatusCode;

use crate::auth::config::AuthConfig;
use crate::oauth_client::OAUTH_BLOCKED_BY_NETWORK_POLICY;

/// Why a browser OAuth connect failed, as the console sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectErrorCode {
    /// The egress network policy refused a discovery, registration, or token host.
    BlockedByNetworkPolicy,
    /// Discovery, registration, or the token exchange failed upstream.
    ProviderUnreachable,
    /// The provider returned an OAuth error to the callback.
    ProviderRefused,
    /// Anything else (permissions, validation, storage).
    Failed,
}

impl ConnectErrorCode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::BlockedByNetworkPolicy => "blocked_by_network_policy",
            Self::ProviderUnreachable => "provider_unreachable",
            Self::ProviderRefused => "provider_refused",
            Self::Failed => "failed",
        }
    }

    /// Classify a handler error. Upstream failures surface as 502 from the
    /// shared OAuth client; a policy denial carries its dedicated message.
    pub(crate) fn classify(error: &(StatusCode, String)) -> Self {
        if error.1 == OAUTH_BLOCKED_BY_NETWORK_POLICY {
            Self::BlockedByNetworkPolicy
        } else if error.0 == StatusCode::BAD_GATEWAY {
            Self::ProviderUnreachable
        } else {
            Self::Failed
        }
    }
}

/// The same-origin path rule every connect `return_to` must pass.
pub(super) fn is_safe_return_to(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
}

/// Query keys the connect result owns; stale ones are dropped from `return_to`
/// so a failure never also reads as `connected`.
const RESULT_PARAMS: &[&str] = &["connected", "connect_error", "provider"];

fn strip_result_params(return_to: &str) -> String {
    let Some((path, query)) = return_to.split_once('?') else {
        return return_to.to_string();
    };
    let kept: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
        .filter(|(key, _)| !RESULT_PARAMS.contains(&key.as_ref()))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    if kept.is_empty() {
        return path.to_string();
    }
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(kept)
        .finish();
    format!("{path}?{query}")
}

/// Build the browser target for a failed connect, or `None` when there is no
/// safe `return_to` (the caller then returns its plain error response).
pub(super) fn connect_error_redirect(
    auth_config: &AuthConfig,
    return_to: Option<&str>,
    provider: &str,
    popup: bool,
    code: ConnectErrorCode,
) -> Option<String> {
    let return_to = return_to.filter(|path| is_safe_return_to(path))?;
    let return_to = strip_result_params(return_to);
    let frontend = auth_config.frontend_url.trim_end_matches('/');
    let encoded_provider = urlencoding::encode(provider);
    let code = code.as_str();
    Some(if popup {
        format!(
            "{frontend}/connection-complete?provider={encoded_provider}&status=error&connect_error={code}&return_to={}",
            urlencoding::encode(&return_to),
        )
    } else {
        let separator = if return_to.contains('?') { '&' } else { '?' };
        format!("{frontend}{return_to}{separator}connect_error={code}&provider={encoded_provider}")
    })
}

/// Turn a browser connect failure into a redirect back to `return_to` when one
/// is safe; otherwise keep the original error. Logs the detail either way.
pub(super) fn redirect_on_connect_error<T>(
    auth_config: &AuthConfig,
    result: Result<T, (StatusCode, String)>,
    return_to: Option<&str>,
    provider: &str,
    popup: bool,
    code_override: Option<ConnectErrorCode>,
) -> Result<Result<T, String>, (StatusCode, String)> {
    let error = match result {
        Ok(value) => return Ok(Ok(value)),
        Err(error) => error,
    };
    let code = code_override.unwrap_or_else(|| ConnectErrorCode::classify(&error));
    match connect_error_redirect(auth_config, return_to, provider, popup, code) {
        Some(target) => {
            tracing::warn!(
                provider,
                status = %error.0,
                error = %error.1,
                connect_error = code.as_str(),
                "OAuth connect failed; returning the browser to its page"
            );
            Ok(Err(target))
        }
        None => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> AuthConfig {
        AuthConfig {
            frontend_url: "https://console.example".into(),
            ..AuthConfig::default()
        }
    }

    #[test]
    fn classifies_policy_upstream_and_other_failures() {
        assert_eq!(
            ConnectErrorCode::classify(&(
                StatusCode::BAD_GATEWAY,
                OAUTH_BLOCKED_BY_NETWORK_POLICY.into()
            )),
            ConnectErrorCode::BlockedByNetworkPolicy
        );
        assert_eq!(
            ConnectErrorCode::classify(&(StatusCode::BAD_GATEWAY, "Bad gateway".into())),
            ConnectErrorCode::ProviderUnreachable
        );
        assert_eq!(
            ConnectErrorCode::classify(&(StatusCode::FORBIDDEN, "nope".into())),
            ConnectErrorCode::Failed
        );
    }

    #[test]
    fn redirects_only_to_a_same_origin_path() {
        for unsafe_target in [
            None,
            Some("https://attacker.example/x"),
            Some("//attacker.example"),
            Some("/\\attacker.example"),
            Some("/path\n"),
        ] {
            assert_eq!(
                connect_error_redirect(
                    &config(),
                    unsafe_target,
                    "mcp_oauth_x",
                    false,
                    ConnectErrorCode::Failed
                ),
                None,
                "{unsafe_target:?}"
            );
        }
        assert_eq!(
            connect_error_redirect(
                &config(),
                Some("/agents/a1?tab=mcp"),
                "mcp_oauth_x",
                false,
                ConnectErrorCode::BlockedByNetworkPolicy
            )
            .as_deref(),
            Some(
                "https://console.example/agents/a1?tab=mcp&connect_error=blocked_by_network_policy&provider=mcp_oauth_x"
            )
        );
    }

    #[test]
    fn drops_a_stale_connected_param_and_supports_popups() {
        assert_eq!(
            connect_error_redirect(
                &config(),
                Some("/settings/connections?connected=mcp_oauth_x"),
                "mcp_oauth_x",
                false,
                ConnectErrorCode::ProviderRefused
            )
            .as_deref(),
            Some(
                "https://console.example/settings/connections?connect_error=provider_refused&provider=mcp_oauth_x"
            )
        );
        assert_eq!(
            connect_error_redirect(
                &config(),
                Some("/chat/s1"),
                "mcp_oauth_x",
                true,
                ConnectErrorCode::ProviderUnreachable
            )
            .as_deref(),
            Some(
                "https://console.example/connection-complete?provider=mcp_oauth_x&status=error&connect_error=provider_unreachable&return_to=%2Fchat%2Fs1"
            )
        );
    }
}
