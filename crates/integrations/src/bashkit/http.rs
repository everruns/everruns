//! Outbound HTTP for the Bashkit shell: egress-routed transport, observational
//! hooks, bot-auth signing, and bound secrets (API keys the shell uses as
//! `$NAME` placeholders and never reads).

use super::egress_transport;
use bashkit::{BashBuilder, NetworkAllowlist, OutputCallback, StreamData};
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::{BoundSecret, SecretAuth};
use std::sync::{Arc, LazyLock};

/// Configure shell HTTP and the bound secrets that ride on it. Returns the
/// redactor every piece of shell output must pass through.
pub(super) async fn configure(
    builder: BashBuilder,
    enable_http: bool,
    context: &ToolContext,
) -> (BashBuilder, OutputRedactor) {
    let builder = configure_http(builder, enable_http, context);
    let bound = bound_secrets(enable_http, context).await;
    (
        install_bound_secrets(builder, &bound),
        OutputRedactor::new(&bound),
    )
}

/// Enable outbound HTTP for curl/wget when the per-capability config opted in
/// AND the execution context provides an egress service.
///
/// bashkit keeps its full HTTP policy pipeline — `allow_all()` retains the
/// private-IP-blocking SSRF precheck whose resolve-then-check result is
/// forwarded as pinned addresses — while connectivity is owned by
/// [`egress_transport::BashkitEgressTransport`], so the merged
/// `NetworkAccessList` and the deployment-wide system allowlist are enforced
/// at the egress boundary for every hop (curl/wget re-dispatch redirects).
/// Empty pins from a failed bashkit DNS precheck fail closed via
/// `require_dns_pinning()` (EVE-1154).
///
/// THREAT[TM-BASH-003]: with `enable_http` off (the default) this function is
/// a no-op and the interpreter has no network path. When on, there is no
/// direct-dial fallback: absent an egress service the shell stays offline
/// rather than opening host-local connectivity.
///
/// Bot-auth request signing mirrors web_fetch: server-wide
/// `BOT_AUTH_SIGNING_KEY_SEED` (+ optional `BOT_AUTH_AGENT_FQDN`,
/// `BOT_AUTH_VALIDITY_SECS`) transparently signs every outbound request
/// before it reaches the transport.
fn configure_http(builder: BashBuilder, enable_http: bool, context: &ToolContext) -> BashBuilder {
    if !enable_http {
        return builder;
    }
    let Some(egress) = context.egress_service.clone() else {
        tracing::warn!(
            capability = "bashkit_shell",
            session_id = %context.session_id,
            "enable_http set but no egress service in context; shell HTTP stays disabled"
        );
        return builder;
    };
    let session_id = context.session_id;
    let mut builder = builder
        .network(NetworkAllowlist::allow_all())
        .http_transport(Arc::new(egress_transport::BashkitEgressTransport::new(
            egress,
            context.network_access.clone(),
        )))
        // Observational HTTP hooks (bashkit-requirements.md): log method and
        // status only — URLs and headers can carry tenant data or secrets.
        .before_http(Box::new(move |ev: bashkit::hooks::HttpRequestEvent| {
            tracing::debug!(
                target: "bashkit.hook",
                capability = "bashkit_shell",
                session_id = %session_id,
                event = "before_http",
                method = %ev.method,
                header_count = ev.headers.len(),
                "outbound http request"
            );
            bashkit::hooks::HookAction::Continue(ev)
        }))
        .after_http(Box::new(move |ev: bashkit::hooks::HttpResponseEvent| {
            tracing::debug!(
                target: "bashkit.hook",
                capability = "bashkit_shell",
                session_id = %session_id,
                event = "after_http",
                status = ev.status,
                "outbound http response"
            );
            bashkit::hooks::HookAction::Continue(ev)
        }));
    if let Some(bot_auth) = bot_auth_config_from_env() {
        builder = builder.bot_auth(bot_auth);
    }
    builder
}

/// Read the server-wide bot-auth signing config once (same env contract as
/// `web_fetch`). Returns a fresh clone per
/// call site because `BashBuilder::bot_auth` takes ownership.
fn bot_auth_config_from_env() -> Option<bashkit::BotAuthConfig> {
    static CONFIG: LazyLock<Option<bashkit::BotAuthConfig>> = LazyLock::new(|| {
        let seed = std::env::var("BOT_AUTH_SIGNING_KEY_SEED").ok()?;
        let mut config = match bashkit::BotAuthConfig::from_base64_seed(&seed) {
            Ok(config) => config,
            Err(e) => {
                tracing::warn!(error = %e, "invalid BOT_AUTH_SIGNING_KEY_SEED, bashkit bot-auth disabled");
                return None;
            }
        };
        if let Ok(fqdn) = std::env::var("BOT_AUTH_AGENT_FQDN") {
            config = config.with_agent_fqdn(&fqdn);
        }
        if let Ok(secs) = std::env::var("BOT_AUTH_VALIDITY_SECS")
            && let Ok(secs) = secs.parse::<u64>()
        {
            config = config.with_validity_secs(secs);
        }
        Some(config)
    });
    CONFIG.clone()
}

/// Bound secrets for this shell call, resolved only when shell HTTP can run.
///
/// THREAT[TM-BASH-030]: values go to Bashkit's credential hook and the
/// output redactor, never into the environment or a tool result. The shell
/// sees `$NAME` as a fresh `bk_placeholder_<hex>` per call, swapped for the
/// value only in the auth header of requests matching the bound URL, after
/// the allowlist check and on every redirect hop (TM-BASH-031). A failed
/// lookup leaves the shell without the keys rather than failing the call.
async fn bound_secrets(enable_http: bool, context: &ToolContext) -> Vec<BoundSecret> {
    if !enable_http || context.egress_service.is_none() {
        return Vec::new();
    }
    let Some(store) = context.storage_store.as_ref() else {
        return Vec::new();
    };
    match store.resolve_bound_secrets(context.session_id).await {
        Ok(secrets) => secrets,
        Err(error) => {
            tracing::warn!(
                capability = "bashkit_shell",
                session_id = %context.session_id,
                error = %error,
                "could not resolve bound secrets; shell HTTP runs without them"
            );
            Vec::new()
        }
    }
}

fn install_bound_secrets(mut builder: BashBuilder, secrets: &[BoundSecret]) -> BashBuilder {
    for secret in secrets {
        let credential = match &secret.info.binding.auth {
            SecretAuth::Bearer => bashkit::Credential::bearer(secret.value.clone()),
            SecretAuth::Header { name } => {
                bashkit::Credential::header(name.clone(), secret.value.clone())
            }
        };
        builder =
            builder.credential_placeholder(&secret.info.name, &secret.info.binding.url, credential);
    }
    builder
}

/// Replaces bound secret values in shell output, so an API that echoes the
/// auth header back still cannot put a key in the transcript (TM-BASH-030).
#[derive(Clone, Default)]
pub(super) struct OutputRedactor {
    values: Arc<Vec<(String, String)>>,
}

impl OutputRedactor {
    fn new(secrets: &[BoundSecret]) -> Self {
        let mut values: Vec<(String, String)> = secrets
            .iter()
            // Very short values would redact ordinary text and protect nothing.
            .filter(|secret| secret.value.len() >= 8)
            .map(|secret| {
                (
                    secret.value.clone(),
                    format!("[REDACTED:{}]", secret.info.name),
                )
            })
            .collect();
        // Longest first, so a value containing another is replaced whole.
        values.sort_by_key(|value| std::cmp::Reverse(value.0.len()));
        Self {
            values: Arc::new(values),
        }
    }

    /// Redact both streams before `inner` sees a chunk. Without bound
    /// secrets the callback is returned as is, so byte output stays exact.
    pub(super) fn wrap(&self, mut inner: OutputCallback) -> OutputCallback {
        if self.values.is_empty() {
            return inner;
        }
        let redactor = self.clone();
        Box::new(move |stdout, stderr| {
            let stdout = StreamData::from(redactor.apply(&stdout.to_string()));
            let stderr = StreamData::from(redactor.apply(&stderr.to_string()));
            inner(&stdout, &stderr)
        })
    }

    pub(super) fn apply(&self, text: &str) -> String {
        let mut text = text.to_string();
        for (value, label) in self.values.iter() {
            if text.contains(value.as_str()) {
                text = text.replace(value.as_str(), label);
            }
        }
        text
    }
}
