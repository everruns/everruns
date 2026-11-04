//! Outbound HTTP for the Bashkit shell: egress-routed transport, observational
//! hooks, bot-auth signing, and bound secrets (API keys the shell uses as
//! `$NAME` placeholders and never reads).

use super::egress_transport;
use bashkit::{BashBuilder, NetworkAllowlist, OutputCallback, StreamData};
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::{BoundSecret, SecretAuth};
use std::sync::Arc;

/// Configure shell HTTP and the bound secrets that ride on it. Returns the
/// redactor every piece of shell output must pass through.
pub(super) async fn configure(
    builder: BashBuilder,
    enable_http: bool,
    context: &ToolContext,
    usage: Option<&serde_json::Value>,
) -> Result<(BashBuilder, OutputRedactor), String> {
    if enable_http
        && let Some(service) = context
            .storage_store
            .as_ref()
            .and_then(|s| s.credential_service())
    {
        use everruns_contracts::runtime::credential_services::CredentialInvocation;
        use everruns_contracts::runtime::resource_access::CredentialUsage;
        let invocation = *context
            .extension::<CredentialInvocation>()
            .ok_or("Shell HTTP credentials require a verified invocation")?;
        let usage: CredentialUsage =
            serde_json::from_value(usage.cloned().unwrap_or(serde_json::json!("service")))
                .map_err(|_| "credential_usage must be user or service")?;
        let metadata = service
            .list(invocation, usage)
            .await
            .map_err(|_| "Could not authorize shell HTTP credentials")?;
        let mut builder = builder;
        let mut references = Vec::new();
        for info in metadata {
            let placeholder = format!(
                "er_credential_{}",
                everruns_contracts::typed_id::SessionId::new()
                    .uuid()
                    .simple()
            );
            builder = builder.env(&info.name, &placeholder);
            references.push((info, placeholder));
        }
        let mut scoped = context.clone();
        scoped.egress_service = Some(Arc::new(super::credential_egress::CredentialEgress {
            inner: context
                .egress_service
                .clone()
                .ok_or("Shell credential requests require platform egress")?,
            service,
            invocation,
            usage,
            references,
        }));
        return Ok((
            configure_http(builder, enable_http, &scoped),
            OutputRedactor::default(),
        ));
    }
    let builder = configure_http(builder, enable_http, context);
    let bound = bound_secrets(enable_http, context).await;
    Ok((
        install_bound_secrets(builder, &bound),
        OutputRedactor::new(&bound),
    ))
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
/// Web Bot Auth signing happens at the egress boundary, like web_fetch
/// (`everruns_core::host::web_bot_auth`), not inside Bashkit.
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
    builder
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
        }))
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
            SecretAuth::FormField { .. } => continue,
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
