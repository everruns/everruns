// LLM Provider service for business logic
//
// On create/update/delete, the LLM resolver cache is invalidated so that
// subsequent model resolutions pick up the new provider config.

use crate::errors::BadRequestError;
use crate::kernel_imports::{
    Caller, Permission, Policy, Rule, everruns_provider::provider::DriverId,
    everruns_provider::provider::ProviderStatus,
};
use crate::services::ProviderResolverService;
use crate::storage::{
    EncryptionService, StorageBackend,
    models::{CreateProviderRow, ProviderRow, UpdateProvider},
};
use anyhow::{Result, anyhow};
use everruns_provider::provider::{Provider, ProviderRequestOptions, ProviderTraceConfig};
use everruns_provider::url_validation::validate_safe_url;
use reqwest::Url;
use std::sync::Arc;
use tracing::error;
use uuid::Uuid;

use crate::api::providers::{CreateProviderRequest, UpdateProviderRequest};

pub const LLM_PROVIDER_VIEW: Policy = Policy {
    id: "provider.view",
    rules: &[Rule::UserHasPermission(Permission::OrgProvidersView)],
};
pub const LLM_PROVIDER_MANAGE: Policy = Policy {
    id: "provider.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgProvidersManage)],
};

pub struct ProviderService {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
    provider_resolver: Option<Arc<ProviderResolverService>>,
}

impl ProviderService {
    pub fn new(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> Self {
        Self {
            db,
            encryption,
            provider_resolver: None,
        }
    }

    pub fn with_resolver(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        resolver: Arc<ProviderResolverService>,
    ) -> Self {
        Self {
            db,
            encryption,
            provider_resolver: Some(resolver),
        }
    }

    /// Invalidate resolver cache after provider mutation.
    async fn invalidate_resolver_cache(&self, org_id: i64) {
        if let Some(ref resolver) = self.provider_resolver {
            resolver.invalidate_cache(org_id).await;
        }
    }

    pub async fn create(&self, caller: &Caller, req: CreateProviderRequest) -> Result<Provider> {
        validate_provider_type(&req.provider_type)?;
        validate_provider_base_url(req.provider_type.clone(), req.base_url.as_deref())?;
        validate_trace_config(req.trace.as_ref())?;
        validate_request_options(req.request_options.as_ref())?;

        // Encrypt API key if provided
        let api_key_encrypted = if let Some(api_key) = &req.api_key {
            let encryption = self
                .encryption
                .as_ref()
                .ok_or_else(|| anyhow!("Encryption not configured. Cannot store API key."))?;
            Some(encryption.encrypt_string(api_key)?)
        } else {
            None
        };

        // Persist only the explicit overrides; driver defaults are applied on
        // read.
        let mut settings_map = serde_json::Map::new();
        if let Some(trace) = req.trace.as_ref() {
            settings_map.insert("trace".to_string(), serde_json::to_value(trace)?);
        }
        if let Some(request_options) = req.request_options.as_ref() {
            settings_map.insert(
                "request_options".to_string(),
                serde_json::to_value(request_options)?,
            );
        }
        let settings =
            (!settings_map.is_empty()).then_some(serde_json::Value::Object(settings_map));

        let input = CreateProviderRow {
            name: req.name,
            provider_type: req.provider_type.to_string(),
            base_url: req.base_url,
            api_key_encrypted,
            settings,
        };

        let row = self.db.create_provider(caller.org_id, input).await?;
        self.invalidate_resolver_cache(caller.org_id).await;
        Ok(Self::row_to_provider(&row))
    }

    pub async fn get(&self, caller: &Caller, id: Uuid) -> Result<Option<Provider>> {
        // EVE-417: log the underlying DB error with org/provider context so
        // operators can diagnose the org-scoped read failures that surface
        // through MCP as a structured `internal: <message>` (and previously as
        // an opaque `callback failed`). The error still propagates unchanged.
        let row = self
            .db
            .get_provider(caller.org_id, id)
            .await
            .inspect_err(|err| {
                error!(
                    org_id = caller.org_id,
                    provider_id = %id,
                    error = %err,
                    "Failed to read llm provider"
                );
            })?;
        Ok(row.as_ref().map(Self::row_to_provider))
    }

    pub async fn list(&self, caller: &Caller) -> Result<Vec<Provider>> {
        // EVE-417: same diagnostic logging as `get`. List failures here are
        // the most common path for org-scoped MCP read regressions.
        let rows = self
            .db
            .list_providers(caller.org_id)
            .await
            .inspect_err(|err| {
                error!(
                    org_id = caller.org_id,
                    error = %err,
                    "Failed to list llm providers"
                );
            })?;
        Ok(rows.iter().map(Self::row_to_provider).collect())
    }

    pub async fn update(
        &self,
        caller: &Caller,
        id: Uuid,
        req: UpdateProviderRequest,
    ) -> Result<Option<Provider>> {
        let existing = match self.db.get_provider(caller.org_id, id).await? {
            Some(row) => row,
            None => return Ok(None),
        };

        // EVE-810 / TM-AUTHZ: a host-managed provider is owned by the embedder.
        // Refuse tenant edits (credentials, base_url, status) with a 403 so a
        // user with `provider.manage` cannot break a row the host is responsible
        // for. `PolicyError` maps to Forbidden at the command boundary.
        if existing.managed {
            return Err(everruns_core::PolicyError::denied(
                "provider_managed",
                "This provider is managed by the host and cannot be modified.",
            )
            .into());
        }

        let provider_type = req
            .provider_type
            .clone()
            .unwrap_or_else(|| existing.provider_type.parse().unwrap_or(DriverId::OpenAI));
        validate_provider_type(&provider_type)?;
        let base_url = req.base_url.as_deref().or(existing.base_url.as_deref());
        validate_provider_base_url(provider_type, base_url)?;
        validate_trace_config(req.trace.as_ref())?;
        validate_request_options(req.request_options.as_ref())?;

        // Encrypt API key if provided
        let api_key_encrypted = if let Some(api_key) = &req.api_key {
            let encryption = self
                .encryption
                .as_ref()
                .ok_or_else(|| anyhow!("Encryption not configured. Cannot store API key."))?;
            Some(encryption.encrypt_string(api_key)?)
        } else {
            None
        };

        // Merge the supplied overrides into existing settings, preserving any
        // other settings keys. `None` on both leaves settings untouched
        // (COALESCE in storage).
        let settings = (req.trace.is_some() || req.request_options.is_some()).then(|| {
            let mut merged = existing.settings.clone();
            if !merged.is_object() {
                merged = serde_json::json!({});
            }
            if let Some(trace) = req.trace.as_ref() {
                merged["trace"] = serde_json::to_value(trace).unwrap_or(serde_json::Value::Null);
            }
            if let Some(request_options) = req.request_options.as_ref() {
                merged["request_options"] =
                    serde_json::to_value(request_options).unwrap_or(serde_json::Value::Null);
            }
            merged
        });

        let input = UpdateProvider {
            name: req.name,
            provider_type: req.provider_type.map(|t| t.to_string()),
            base_url: req.base_url,
            api_key_encrypted,
            status: req.status.map(|s| match s {
                ProviderStatus::Active => "active".to_string(),
                ProviderStatus::Disabled => "disabled".to_string(),
            }),
            settings,
        };

        let row = self.db.update_provider(caller.org_id, id, input).await?;
        if row.is_some() {
            self.invalidate_resolver_cache(caller.org_id).await;
        }
        Ok(row.as_ref().map(Self::row_to_provider))
    }

    pub async fn delete(&self, caller: &Caller, id: Uuid) -> Result<bool> {
        // EVE-810 / TM-AUTHZ: refuse deletion of a host-managed provider. Fetch
        // first so the managed check runs before any mutation; a missing row
        // falls through to `delete_provider` returning false (404 at the command
        // boundary), preserving existing not-found semantics.
        if let Some(existing) = self.db.get_provider(caller.org_id, id).await?
            && existing.managed
        {
            return Err(everruns_core::PolicyError::denied(
                "provider_managed",
                "This provider is managed by the host and cannot be deleted.",
            )
            .into());
        }

        let deleted = self.db.delete_provider(caller.org_id, id).await?;
        if deleted {
            self.invalidate_resolver_cache(caller.org_id).await;
        }
        Ok(deleted)
    }

    fn row_to_provider(row: &ProviderRow) -> Provider {
        let provider_type = row.provider_type.parse().unwrap_or(DriverId::OpenAI);
        // api_key_set reflects the database only. The tenant resolver is
        // fail-closed and never reads process env at execution time (EVE-511),
        // so reporting an env-derived key here would mislead the UI. In
        // single-tenant/dev deployments DEFAULT_*_API_KEY values are
        // materialized into the provider row at startup (see
        // `seed::seed_default_provider_keys_from_env`), which sets
        // `row.api_key_set` through the normal path.
        let api_key_set = row.api_key_set;
        let trace = resolve_trace_config(&provider_type, &row.settings);
        // Absent or malformed options read back as "nothing configured"; the
        // runtime resolver applies the same fallback.
        let request_options =
            crate::services::provider_resolver::provider_request_options(&row.settings);
        let request_options = (!request_options.is_empty()).then_some(request_options);

        Provider {
            id: row.id,
            name: row.name.clone(),
            provider_type,
            base_url: row.base_url.clone(),
            api_key_set,
            status: match row.status.as_str() {
                "active" => ProviderStatus::Active,
                _ => ProviderStatus::Disabled,
            },
            managed: row.managed,
            last_synced_at: row.last_synced_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
            trace,
            request_options,
        }
    }
}

/// Resolve a provider's trace-link configuration by overlaying its stored
/// override (`settings.trace`) onto the driver's default templates. Returns
/// `None` when neither the driver nor the org provides anything to link to, so
/// the UI shows no trace affordance.
fn resolve_trace_config(
    provider_type: &DriverId,
    settings: &serde_json::Value,
) -> Option<ProviderTraceConfig> {
    let (default_generation, default_session) = provider_type.default_trace_templates();

    // A stored override may be partial; missing fields fall back to defaults.
    let stored: Option<ProviderTraceConfig> = settings
        .get("trace")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    let enabled = stored.as_ref().map(|t| t.enabled).unwrap_or(false);
    let generation_url_template = stored
        .as_ref()
        .and_then(|t| t.generation_url_template.clone())
        .or(default_generation);
    let session_url_template = stored
        .as_ref()
        .and_then(|t| t.session_url_template.clone())
        .or(default_session);

    if !enabled && generation_url_template.is_none() && session_url_template.is_none() {
        return None;
    }

    Some(ProviderTraceConfig {
        enabled,
        generation_url_template,
        session_url_template,
    })
}

/// Reject request options a connection must not carry: too many headers, an
/// empty or syntactically invalid header name, an oversized value, or a
/// connection-level header whose meaning belongs to the transport rather than
/// the caller. Drivers drop the last group anyway; failing here tells the
/// operator instead of silently ignoring what they typed.
fn validate_request_options(options: Option<&ProviderRequestOptions>) -> Result<()> {
    let Some(options) = options else {
        return Ok(());
    };
    if options.headers.len() > MAX_PROVIDER_REQUEST_HEADERS {
        return Err(BadRequestError::new(format!(
            "At most {MAX_PROVIDER_REQUEST_HEADERS} custom headers are allowed"
        ))
        .into());
    }
    for header in &options.headers {
        let name = header.name.trim();
        if name.is_empty() {
            return Err(BadRequestError::new("Header name cannot be empty").into());
        }
        if reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_err() {
            return Err(BadRequestError::new(format!("Invalid header name: {name}")).into());
        }
        if PROTECTED_PROVIDER_HEADERS
            .iter()
            .any(|protected| name.eq_ignore_ascii_case(protected))
        {
            return Err(BadRequestError::new(format!(
                "Header '{name}' is controlled by the transport and cannot be set"
            ))
            .into());
        }
        if reqwest::header::HeaderValue::from_str(&header.value).is_err() {
            return Err(BadRequestError::new(format!("Invalid value for header '{name}'")).into());
        }
        if header.value.len() > MAX_PROVIDER_REQUEST_HEADER_VALUE_LEN {
            return Err(BadRequestError::new(format!(
                "Value for header '{name}' exceeds {MAX_PROVIDER_REQUEST_HEADER_VALUE_LEN} characters"
            ))
            .into());
        }
    }
    Ok(())
}

/// Bounds on stored custom headers. These are sent on every request to the
/// provider, so a runaway list is a per-call cost, not just a storage one.
const MAX_PROVIDER_REQUEST_HEADERS: usize = 16;
const MAX_PROVIDER_REQUEST_HEADER_VALUE_LEN: usize = 2048;

/// Connection-level headers the transport owns. Mirrors the driver-side list in
/// `everruns_provider::driver_helpers::merge_request_headers`.
const PROTECTED_PROVIDER_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "upgrade",
];

fn validate_trace_config(trace: Option<&ProviderTraceConfig>) -> Result<()> {
    let Some(trace) = trace else {
        return Ok(());
    };

    validate_trace_template(trace.generation_url_template.as_deref())
        .map_err(|e| anyhow!("Invalid generation trace URL template: {e}"))?;
    validate_trace_template(trace.session_url_template.as_deref())
        .map_err(|e| anyhow!("Invalid session trace URL template: {e}"))?;

    Ok(())
}

fn validate_trace_template(template: Option<&str>) -> Result<()> {
    let Some(template) = template else {
        return Ok(());
    };
    let rendered = render_trace_template_for_validation(template)?;
    let parsed = validate_safe_url(&rendered)?;
    if parsed.scheme() != "https" {
        return Err(BadRequestError::new("Trace URL templates must use HTTPS").into());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| BadRequestError::new("Trace URL templates must include a host"))?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host.ends_with(".local") || host.ends_with(".internal") {
        return Err(BadRequestError::new("Trace URL template host is not allowed").into());
    }
    Ok(())
}

fn render_trace_template_for_validation(template: &str) -> Result<String> {
    let mut rendered = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(start) = rest.find('{') {
        rendered.push_str(&rest[..start]);
        rest = &rest[start + 1..];
        let end = rest.find('}').ok_or_else(|| {
            BadRequestError::new("Trace URL template contains an unclosed placeholder")
        })?;
        let placeholder = &rest[..end];
        match placeholder {
            "response_id" | "session_id" | "turn_id" | "model" => rendered.push_str("placeholder"),
            _ => {
                return Err(BadRequestError::new(format!(
                    "Unsupported trace URL template placeholder: {{{placeholder}}}"
                ))
                .into());
            }
        }
        rest = &rest[end + 1..];
    }

    if rest.contains('}') {
        return Err(
            BadRequestError::new("Trace URL template contains an unopened placeholder").into(),
        );
    }

    rendered.push_str(rest);
    Ok(rendered)
}

fn validate_provider_type(provider_type: &DriverId) -> Result<()> {
    let raw = provider_type.as_str();
    if raw.trim().is_empty() {
        return Err(BadRequestError::new("Provider type cannot be empty").into());
    }
    // Keep this defense for already-constructed values. HTTP deserialization
    // rejects padding before normalization, so malformed wire ids cannot reach
    // persistence as canonical-but-unintended names.
    if raw != raw.trim() {
        return Err(BadRequestError::new(
            "Provider type cannot have leading or trailing whitespace",
        )
        .into());
    }
    Ok(())
}

fn validate_provider_base_url(provider_type: DriverId, base_url: Option<&str>) -> Result<()> {
    let parsed = match base_url {
        Some(url) => Some(validate_safe_url(url).map_err(|e| anyhow!("Invalid base URL: {e}"))?),
        None => None,
    };

    if provider_type == DriverId::AzureOpenAI {
        let parsed = parsed.ok_or_else(|| {
            anyhow!(
                "Invalid base URL: Azure OpenAI providers require a base URL ending in /openai/v1 on an Azure host"
            )
        })?;
        validate_azure_openai_base_url(&parsed).map_err(|e| anyhow!("Invalid base URL: {e}"))?;
    }

    Ok(())
}

fn validate_azure_openai_base_url(url: &Url) -> Result<()> {
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("Azure OpenAI base URL must include a host"))?
        .to_ascii_lowercase();

    let valid_host =
        host.ends_with(".openai.azure.com") || host.ends_with(".services.ai.azure.com");
    if !valid_host {
        return Err(anyhow!(
            "Azure OpenAI base URL must use *.openai.azure.com or *.services.ai.azure.com"
        ));
    }

    let path = url.path().trim_end_matches('/');
    let valid_path = matches!(path, "/openai/v1" | "/openai/v1/responses");
    if !valid_path {
        return Err(anyhow!(
            "Azure OpenAI base URL must point to /openai/v1 or /openai/v1/responses"
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_PROVIDER_REQUEST_HEADERS, ProviderRequestOptions, resolve_trace_config,
        validate_provider_base_url, validate_provider_type, validate_request_options,
        validate_trace_config,
    };
    use crate::errors::BadRequestError;
    use crate::kernel_imports::{DriverId, ProviderTraceConfig};
    use everruns_provider::url_validation::validate_safe_url;

    // ---- Trace config resolution (provider trace links) ----

    #[test]
    fn trace_openrouter_defaults_present_but_disabled_without_override() {
        // OpenRouter ships default templates, but links stay off until an admin
        // opts in, so the resolved config carries the templates with enabled=false.
        let trace =
            resolve_trace_config(&DriverId::OpenRouter, &serde_json::json!({})).expect("some");
        assert!(!trace.enabled);
        assert_eq!(
            trace.generation_url_template.as_deref(),
            Some("https://openrouter.ai/logs?id={response_id}")
        );
        assert_eq!(
            trace.session_url_template.as_deref(),
            Some("https://openrouter.ai/logs")
        );
    }

    #[test]
    fn trace_none_for_driver_without_defaults_or_override() {
        assert!(resolve_trace_config(&DriverId::OpenAI, &serde_json::json!({})).is_none());
    }

    #[test]
    fn trace_override_enables_and_keeps_driver_default_templates() {
        let settings = serde_json::json!({ "trace": { "enabled": true } });
        let trace = resolve_trace_config(&DriverId::OpenRouter, &settings).expect("some");
        assert!(trace.enabled);
        // Missing override fields fall back to the driver defaults.
        assert_eq!(
            trace.generation_url_template.as_deref(),
            Some("https://openrouter.ai/logs?id={response_id}")
        );
    }

    #[test]
    fn trace_override_template_wins_for_any_driver() {
        // A generic provider with no driver default can still configure links.
        let settings = serde_json::json!({
            "trace": {
                "enabled": true,
                "session_url_template": "https://langfuse.example.com/s/{session_id}"
            }
        });
        let trace = resolve_trace_config(&DriverId::OpenAI, &settings).expect("some");
        assert!(trace.enabled);
        assert_eq!(
            trace.session_url_template.as_deref(),
            Some("https://langfuse.example.com/s/{session_id}")
        );
        assert!(trace.generation_url_template.is_none());
    }

    // ---- SSRF prevention tests (EVE-69) ----

    #[test]
    fn trace_template_validation_accepts_https_public_templates() {
        let trace = ProviderTraceConfig {
            enabled: true,
            generation_url_template: Some(
                "https://openrouter.ai/logs?id={response_id}&model={model}".to_string(),
            ),
            session_url_template: Some("https://logs.example.com/s/{session_id}".to_string()),
        };

        assert!(validate_trace_config(Some(&trace)).is_ok());
    }

    #[test]
    fn trace_template_validation_rejects_http() {
        let trace = ProviderTraceConfig {
            enabled: true,
            generation_url_template: Some("http://logs.example.com/{response_id}".to_string()),
            session_url_template: None,
        };

        let err = validate_trace_config(Some(&trace)).unwrap_err();
        assert!(err.to_string().contains("must use HTTPS"));
    }

    #[test]
    fn trace_template_validation_rejects_internal_hosts() {
        for template in [
            "https://127.0.0.1/{response_id}",
            "https://10.0.0.1/{response_id}",
            "https://169.254.169.254/{response_id}",
            "https://router.local/{response_id}",
        ] {
            let trace = ProviderTraceConfig {
                enabled: true,
                generation_url_template: Some(template.to_string()),
                session_url_template: None,
            };
            assert!(
                validate_trace_config(Some(&trace)).is_err(),
                "expected {template} to be rejected"
            );
        }
    }

    // ---- Request options (custom headers, diagnostics) ----

    fn header(name: &str, value: &str) -> everruns_provider::provider::ProviderRequestHeader {
        everruns_provider::provider::ProviderRequestHeader {
            name: name.to_string(),
            value: value.to_string(),
        }
    }

    #[test]
    fn request_options_validation_accepts_ordinary_headers() {
        let options = ProviderRequestOptions {
            headers: vec![header("x-gateway-tenant", "acme")],
            cache_diagnostics: true,
        };
        assert!(validate_request_options(Some(&options)).is_ok());
    }

    #[test]
    fn request_options_validation_rejects_transport_owned_and_malformed_headers() {
        for bad in [
            header("Host", "evil.example"),
            header("bad name", "v"),
            header("", "v"),
        ] {
            let options = ProviderRequestOptions {
                headers: vec![bad.clone()],
                cache_diagnostics: false,
            };
            assert!(
                validate_request_options(Some(&options)).is_err(),
                "expected header {:?} to be rejected",
                bad.name
            );
        }
    }

    #[test]
    fn request_options_validation_rejects_too_many_headers() {
        let options = ProviderRequestOptions {
            headers: (0..MAX_PROVIDER_REQUEST_HEADERS + 1)
                .map(|i| header(&format!("x-h{i}"), "v"))
                .collect(),
            cache_diagnostics: false,
        };
        assert!(validate_request_options(Some(&options)).is_err());
    }

    #[test]
    fn stored_request_options_read_back_and_absent_ones_stay_none() {
        let settings = serde_json::json!({
            "request_options": {
                "headers": [{"name": "x-gateway-tenant", "value": "acme"}],
                "cache_diagnostics": true
            }
        });
        let options = crate::services::provider_resolver::provider_request_options(&settings);
        assert!(options.cache_diagnostics);
        assert_eq!(
            options.header_pairs(),
            vec![("x-gateway-tenant".to_string(), "acme".to_string())]
        );

        // A malformed blob must not take the provider offline.
        let broken = serde_json::json!({ "request_options": "nonsense" });
        assert!(crate::services::provider_resolver::provider_request_options(&broken).is_empty());
    }

    #[test]
    fn provider_type_rejects_empty_external_id() {
        let err = validate_provider_type(&DriverId::external("")).unwrap_err();
        assert!(err.to_string().contains("Provider type cannot be empty"));
        assert!(err.downcast_ref::<BadRequestError>().is_some());
    }

    #[test]
    fn provider_type_rejects_whitespace_external_id() {
        let err = validate_provider_type(&DriverId::external(" \t\n")).unwrap_err();
        assert!(err.to_string().contains("Provider type cannot be empty"));
        assert!(err.downcast_ref::<BadRequestError>().is_some());
    }

    #[test]
    fn provider_type_rejects_padded_external_id() {
        let result = serde_json::from_str::<DriverId>(r#"" openai ""#);
        assert!(result.is_err());
    }

    #[test]
    fn provider_type_accepts_external_id() {
        assert!(validate_provider_type(&DriverId::external("custom-driver")).is_ok());
    }

    #[test]
    fn create_rejects_localhost_base_url() {
        let url = "https://127.0.0.1/v1";
        let err = validate_safe_url(url).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("Blocked host"),
            "expected blocked host error, got: {msg}"
        );
    }

    #[test]
    fn create_rejects_private_ip_base_url() {
        let url = "https://10.0.0.1/v1";
        let err = validate_safe_url(url).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("Blocked host"),
            "expected blocked host error, got: {msg}"
        );
    }

    #[test]
    fn create_rejects_metadata_endpoint_base_url() {
        let url = "https://169.254.169.254/latest/meta-data/";
        let err = validate_safe_url(url).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("Blocked host"),
            "expected blocked host error, got: {msg}"
        );
    }

    #[test]
    fn create_accepts_valid_public_base_url() {
        assert!(validate_safe_url("https://api.openai.com/v1").is_ok());
        assert!(validate_safe_url("https://api.anthropic.com/v1").is_ok());
        assert!(validate_safe_url("https://resource.openai.azure.com/openai/v1").is_ok());
    }

    #[test]
    fn azure_openai_requires_base_url() {
        let err = validate_provider_base_url(DriverId::AzureOpenAI, None).unwrap_err();
        assert!(
            err.to_string()
                .contains("Invalid base URL: Azure OpenAI providers require a base URL")
        );
    }

    #[test]
    fn azure_openai_rejects_non_azure_hosts() {
        let err =
            validate_provider_base_url(DriverId::AzureOpenAI, Some("https://api.openai.com/v1"))
                .unwrap_err();
        assert!(err.to_string().contains("*.openai.azure.com"));
    }

    #[test]
    fn azure_openai_accepts_supported_domains() {
        assert!(
            validate_provider_base_url(
                DriverId::AzureOpenAI,
                Some("https://resource.openai.azure.com/openai/v1"),
            )
            .is_ok()
        );
        assert!(
            validate_provider_base_url(
                DriverId::AzureOpenAI,
                Some("https://resource.services.ai.azure.com/openai/v1/responses"),
            )
            .is_ok()
        );
    }

    #[test]
    fn azure_openai_rejects_chat_completions_base_url() {
        let err = validate_provider_base_url(
            DriverId::AzureOpenAI,
            Some("https://resource.openai.azure.com/openai/v1/chat/completions"),
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("/openai/v1 or /openai/v1/responses")
        );
    }

    // ---- Host-managed provider enforcement (EVE-810) ----

    mod managed {
        use crate::api::providers::UpdateProviderRequest;
        use crate::domains::providers::ProviderService;
        use crate::kernel_imports::{Caller, OrgRole, PolicyError};
        use crate::storage::StorageBackend;
        use crate::storage::models::CreateProviderRow;
        use std::sync::Arc;
        use uuid::Uuid;

        fn caller(org_id: i64) -> Caller {
            Caller {
                org_id,
                org_public_id: format!("org_{org_id:032}"),
                user_id: None,
                role: OrgRole::Owner,
                is_platform_user: false,
                is_internal: false,
            }
        }

        fn name_update(name: &str) -> UpdateProviderRequest {
            UpdateProviderRequest {
                name: Some(name.to_string()),
                provider_type: None,
                base_url: None,
                api_key: None,
                credentials: None,
                status: None,
                trace: None,
                request_options: None,
            }
        }

        async fn seed_provider(db: &StorageBackend, org_id: i64, managed: bool) -> Uuid {
            let row = db
                .create_provider(
                    org_id,
                    CreateProviderRow {
                        name: "Gateway".to_string(),
                        provider_type: "openai".to_string(),
                        base_url: None,
                        api_key_encrypted: None,
                        settings: None,
                    },
                )
                .await
                .unwrap();
            let id = row.id.uuid();
            if managed {
                assert!(db.set_provider_managed(org_id, id, true).await.unwrap());
            }
            id
        }

        #[tokio::test]
        async fn update_of_managed_provider_is_forbidden() {
            let db = Arc::new(StorageBackend::in_memory());
            let org_id = 1;
            let id = seed_provider(&db, org_id, true).await;
            let service = ProviderService::new(db.clone(), None);

            let err = service
                .update(&caller(org_id), id, name_update("renamed"))
                .await
                .unwrap_err();
            assert!(
                err.downcast_ref::<PolicyError>().is_some(),
                "expected PolicyError (403), got: {err}"
            );
            // GET still works and exposes the managed flag.
            let got = service.get(&caller(org_id), id).await.unwrap().unwrap();
            assert!(got.managed);
            // The name was not changed.
            assert_eq!(got.name, "Gateway");
        }

        #[tokio::test]
        async fn delete_of_managed_provider_is_forbidden() {
            let db = Arc::new(StorageBackend::in_memory());
            let org_id = 1;
            let id = seed_provider(&db, org_id, true).await;
            let service = ProviderService::new(db.clone(), None);

            let err = service.delete(&caller(org_id), id).await.unwrap_err();
            assert!(
                err.downcast_ref::<PolicyError>().is_some(),
                "expected PolicyError (403), got: {err}"
            );
            // Row survives the rejected delete.
            assert!(db.get_provider(org_id, id).await.unwrap().is_some());
        }

        #[tokio::test]
        async fn unmanaged_provider_stays_editable_and_deletable() {
            let db = Arc::new(StorageBackend::in_memory());
            let org_id = 1;
            let id = seed_provider(&db, org_id, false).await;
            let service = ProviderService::new(db.clone(), None);

            let updated = service
                .update(&caller(org_id), id, name_update("renamed"))
                .await
                .unwrap()
                .expect("provider exists");
            assert_eq!(updated.name, "renamed");
            assert!(!updated.managed);

            assert!(service.delete(&caller(org_id), id).await.unwrap());
            assert!(db.get_provider(org_id, id).await.unwrap().is_none());
        }
    }
}
