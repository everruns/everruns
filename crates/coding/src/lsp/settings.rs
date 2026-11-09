//! The capability's config: which language servers exist and how long to wait
//! for them.

use super::manager::{
    DEFAULT_DIAGNOSTICS_WAIT_MS, DEFAULT_REQUEST_TIMEOUT_MS, LspManager, MAX_DIAGNOSTICS_WAIT_MS,
    MAX_REQUEST_TIMEOUT_MS, ServerSpec, default_server_specs,
};
use crate::host::CodeHost;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

/// User-facing capability config.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LspSettings {
    /// Per-language overrides keyed by server key (`rust`, `typescript`,
    /// `python`, `go`, `clangd`, or any new key with a `command`).
    #[serde(default)]
    servers: BTreeMap<String, ServerOverride>,
    #[serde(default)]
    request_timeout_ms: Option<u64>,
    #[serde(default)]
    diagnostics_wait_ms: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ServerOverride {
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Option<Vec<String>>,
    #[serde(default)]
    extensions: Option<Vec<String>>,
    /// `false` disables a built-in server without replacing it.
    #[serde(default)]
    enabled: Option<bool>,
}

pub(super) fn parse_settings(config: &Value) -> Result<LspSettings, String> {
    if config.is_null() {
        return Ok(LspSettings::default());
    }
    serde_json::from_value(config.clone()).map_err(|err| format!("invalid lsp config: {err}"))
}

pub(super) fn resolve_specs(settings: &LspSettings) -> Result<Vec<ServerSpec>, String> {
    let mut specs = default_server_specs();
    for (key, over) in &settings.servers {
        if over.enabled == Some(false) {
            specs.retain(|spec| &spec.key != key);
            continue;
        }
        if let Some(spec) = specs.iter_mut().find(|spec| &spec.key == key) {
            if let Some(command) = &over.command {
                spec.command = command.clone();
            }
            if let Some(args) = &over.args {
                spec.args = args.clone();
            }
            if let Some(extensions) = &over.extensions {
                spec.extensions = normalized_extensions(extensions);
            }
        } else {
            let command = over.command.clone().ok_or_else(|| {
                format!("lsp server `{key}` is not a built-in; it requires a `command`")
            })?;
            let extensions = over.extensions.clone().ok_or_else(|| {
                format!("lsp server `{key}` is not a built-in; it requires `extensions`")
            })?;
            specs.push(ServerSpec {
                key: key.clone(),
                command,
                args: over.args.clone().unwrap_or_default(),
                extensions: normalized_extensions(&extensions),
            });
        }
    }
    Ok(specs)
}

fn normalized_extensions(extensions: &[String]) -> Vec<String> {
    extensions
        .iter()
        .map(|ext| ext.trim_start_matches('.').to_ascii_lowercase())
        .filter(|ext| !ext.is_empty())
        .collect()
}

pub(super) fn build_manager(
    host: Arc<dyn CodeHost>,
    scope: String,
    config: &Value,
) -> Result<LspManager, String> {
    let settings = parse_settings(config)?;
    let specs = resolve_specs(&settings)?;
    let request_timeout = Duration::from_millis(
        settings
            .request_timeout_ms
            .unwrap_or(DEFAULT_REQUEST_TIMEOUT_MS)
            .clamp(1_000, MAX_REQUEST_TIMEOUT_MS),
    );
    let diagnostics_wait = Duration::from_millis(
        settings
            .diagnostics_wait_ms
            .unwrap_or(DEFAULT_DIAGNOSTICS_WAIT_MS)
            .clamp(100, MAX_DIAGNOSTICS_WAIT_MS),
    );
    Ok(LspManager::new(
        host,
        scope,
        specs,
        request_timeout,
        diagnostics_wait,
    ))
}
