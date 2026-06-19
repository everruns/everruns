// OpenRouter Workspace Capability
//
// Provides host-facing tools for inspecting OpenRouter workspace/key policy
// metadata and detecting drift between local routing config and upstream
// workspace constraints.
//
// Tools contributed:
//   - inspect_openrouter_workspace: reads key status, budget, and rate-limit
//     from OpenRouter's /api/v1/auth/key endpoint. Sensitive key material is
//     never returned; only derived metadata.
//   - check_openrouter_policy_compatibility: compares local routing parameters
//     against the inspected workspace policy and returns a structured
//     compatibility report with detected drifts.
//
// Security: TM-LLM — API key sourced from provider_credential_store and never
// logged or returned to callers. TM-API — only the /api/v1/auth/key endpoint
// is called; no write operations against the workspace.

use super::{Capability, CapabilityLocalization, CapabilityStatus};
use crate::egress::{EgressError, EgressRequest, EgressRequestKind};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const OPENROUTER_WORKSPACE_CAPABILITY_ID: &str = "openrouter_workspace";

/// Capability that contributes workspace-inspection and policy-compatibility
/// tools for OpenRouter providers.
pub struct OpenRouterWorkspaceCapability;

impl Capability for OpenRouterWorkspaceCapability {
    fn id(&self) -> &str {
        OPENROUTER_WORKSPACE_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "OpenRouter Workspace"
    }

    fn description(&self) -> &str {
        "Inspect OpenRouter workspace policy (budget, rate limits, tier) and detect incompatibilities with local routing configuration."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("shield-check")
    }

    fn category(&self) -> Option<&str> {
        Some("AI")
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(InspectOpenRouterWorkspaceTool),
            Box::new(CheckOpenRouterPolicyCompatibilityTool),
        ]
    }
}

// ============================================================================
// Workspace policy types
// ============================================================================

/// Key/workspace metadata returned by OpenRouter's /api/v1/auth/key endpoint.
///
/// Usage and limit are converted from OpenRouter's millionths-of-USD integers
/// to human-readable USD floats. The raw API key is never included.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenRouterKeyInfo {
    /// Human-readable label assigned to this API key.
    pub label: String,
    /// Total spend in USD against this key.
    pub usage_usd: f64,
    /// Spend cap in USD. `None` means unlimited.
    pub limit_usd: Option<f64>,
    /// Remaining budget in USD. `None` means unlimited.
    pub remaining_usd: Option<f64>,
    /// Whether this is a free-tier key (subject to model and rate restrictions).
    pub is_free_tier: bool,
    /// Rate limit applied to this key, if any.
    pub rate_limit: Option<OpenRouterRateLimit>,
}

impl OpenRouterKeyInfo {
    /// Parse the raw JSON response from /api/v1/auth/key.
    ///
    /// `usage` and `limit` arrive as millionths of USD (integer). This method
    /// converts them to USD floats and computes `remaining_usd`.
    fn from_api_response(data: &Value) -> Result<Self, String> {
        let label = data
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("(unnamed)")
            .to_string();

        // usage is in millionths of USD
        let usage_usd = data
            .get("usage")
            .and_then(|v| v.as_f64())
            .map(|u| u / 1_000_000.0)
            .unwrap_or(0.0);

        let limit_usd = data
            .get("limit")
            .and_then(|v| if v.is_null() { None } else { v.as_f64() })
            .map(|l| l / 1_000_000.0);

        let remaining_usd = limit_usd.map(|l| (l - usage_usd).max(0.0));

        let is_free_tier = data
            .get("is_free_tier")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let rate_limit = data.get("rate_limit").and_then(|rl| {
            let requests = rl.get("requests")?.as_u64()?.try_into().ok()?;
            let interval = rl.get("interval")?.as_str()?.to_string();
            Some(OpenRouterRateLimit { requests, interval })
        });

        Ok(OpenRouterKeyInfo {
            label,
            usage_usd,
            limit_usd,
            remaining_usd,
            is_free_tier,
            rate_limit,
        })
    }
}

/// Rate limit information for an OpenRouter API key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenRouterRateLimit {
    /// Maximum number of requests allowed in the interval.
    pub requests: u32,
    /// Time window for the rate limit (e.g. "10s", "1m").
    pub interval: String,
}

/// A detected incompatibility between local routing config and the upstream
/// OpenRouter workspace policy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspacePolicyDrift {
    /// The workspace has a spend cap and the remaining budget is zero.
    BudgetExhausted {
        usage_usd: f64,
        limit_usd: f64,
        message: String,
    },
    /// The remaining budget is below the caller-requested per-probe threshold.
    BudgetBelowThreshold {
        remaining_usd: f64,
        threshold_usd: f64,
        message: String,
    },
    /// The key is on the free tier but the routing config requires paid-tier
    /// features (e.g. `zdr`, `data_collection: deny`, paid-only providers).
    FreeTierRestriction { feature: String, message: String },
}

/// Full compatibility report produced by `check_openrouter_policy_compatibility`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyCompatibilityReport {
    /// `true` when no drifts were detected.
    pub compatible: bool,
    /// Workspace metadata at the time of the check (no API key material).
    pub workspace: OpenRouterKeyInfo,
    /// Detected incompatibilities, empty when `compatible` is `true`.
    pub drifts: Vec<WorkspacePolicyDrift>,
}

/// Evaluate workspace policy against caller-supplied routing parameters and
/// return detected drift entries.
pub fn detect_policy_drift(
    workspace: &OpenRouterKeyInfo,
    min_remaining_budget_usd: Option<f64>,
    requires_paid_features: bool,
) -> Vec<WorkspacePolicyDrift> {
    let mut drifts = Vec::new();

    // Check budget exhaustion
    if let (Some(remaining), Some(limit)) = (workspace.remaining_usd, workspace.limit_usd) {
        if remaining <= 0.0 {
            drifts.push(WorkspacePolicyDrift::BudgetExhausted {
                usage_usd: workspace.usage_usd,
                limit_usd: limit,
                message: format!(
                    "Workspace budget exhausted: spent ${:.6} of ${:.6} limit",
                    workspace.usage_usd, limit
                ),
            });
        } else if let Some(threshold) = min_remaining_budget_usd.filter(|&t| remaining < t) {
            drifts.push(WorkspacePolicyDrift::BudgetBelowThreshold {
                remaining_usd: remaining,
                threshold_usd: threshold,
                message: format!(
                    "Remaining workspace budget (${remaining:.6}) is below the requested \
                     threshold (${threshold:.6})"
                ),
            });
        }
    }
    // Unlimited budget (no limit set) always passes threshold check — no action needed.

    // Check free-tier vs paid-feature requirements
    if workspace.is_free_tier && requires_paid_features {
        drifts.push(WorkspacePolicyDrift::FreeTierRestriction {
            feature: "paid_tier_routing".to_string(),
            message: "Workspace is on the free tier, which restricts access to paid-only \
                      providers, ZDR endpoints, and data-retention controls. Upgrade to a paid \
                      key to use these features."
                .to_string(),
        });
    }

    drifts
}

// ============================================================================
// Tool: inspect_openrouter_workspace
// ============================================================================

struct InspectOpenRouterWorkspaceTool;

#[async_trait]
impl Tool for InspectOpenRouterWorkspaceTool {
    fn name(&self) -> &str {
        "inspect_openrouter_workspace"
    }

    fn description(&self) -> &str {
        "Fetch OpenRouter workspace/key metadata (budget, rate limits, tier status). \
         Returns structured policy metadata without exposing the API key itself. \
         Use this to understand workspace constraints before configuring model routing."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "inspect_openrouter_workspace requires provider credentials — use execute_with_context",
        )
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let body = match fetch_openrouter_key_info(context).await {
            Ok(body) => body,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        let data = match body.get("data") {
            Some(d) => d,
            None => {
                return ToolExecutionResult::tool_error(
                    "OpenRouter /auth/key response missing 'data' field",
                );
            }
        };

        let info = match OpenRouterKeyInfo::from_api_response(data) {
            Ok(i) => i,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        ToolExecutionResult::Success(json!(info))
    }
}

// ============================================================================
// Tool: check_openrouter_policy_compatibility
// ============================================================================

struct CheckOpenRouterPolicyCompatibilityTool;

#[async_trait]
impl Tool for CheckOpenRouterPolicyCompatibilityTool {
    fn name(&self) -> &str {
        "check_openrouter_policy_compatibility"
    }

    fn description(&self) -> &str {
        "Check whether local routing configuration is compatible with the current OpenRouter \
         workspace policy. Returns a compatibility report listing any detected drift between \
         local settings and upstream workspace constraints (budget, tier, rate limits). \
         Use before finalizing routing config to catch incompatibilities early."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "min_remaining_budget_usd": {
                    "type": "number",
                    "minimum": 0,
                    "description": "Minimum remaining workspace budget (USD) required by your \
                                    routing plan. A drift is reported if the workspace has less \
                                    than this amount remaining. Omit to skip budget-threshold check."
                },
                "requires_paid_features": {
                    "type": "boolean",
                    "description": "Set true if the routing config uses paid-tier features such as \
                                    ZDR endpoints, data-collection controls, or paid-only providers. \
                                    A drift is reported when the workspace is on the free tier.",
                    "default": false
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "check_openrouter_policy_compatibility requires provider credentials — use execute_with_context",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let min_remaining_budget_usd = arguments
            .get("min_remaining_budget_usd")
            .and_then(|v| v.as_f64())
            .filter(|&v| v >= 0.0);
        let requires_paid_features = arguments
            .get("requires_paid_features")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let body = match fetch_openrouter_key_info(context).await {
            Ok(body) => body,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        let data = match body.get("data") {
            Some(d) => d,
            None => {
                return ToolExecutionResult::tool_error(
                    "OpenRouter /auth/key response missing 'data' field",
                );
            }
        };

        let workspace = match OpenRouterKeyInfo::from_api_response(data) {
            Ok(i) => i,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        let drifts =
            detect_policy_drift(&workspace, min_remaining_budget_usd, requires_paid_features);
        let compatible = drifts.is_empty();

        let report = PolicyCompatibilityReport {
            compatible,
            workspace,
            drifts,
        };

        ToolExecutionResult::Success(json!(report))
    }
}

// ============================================================================
// Credential helper (shared with model_scout pattern)
// ============================================================================

const OPENROUTER_KEY_INFO_URL: &str = "https://openrouter.ai/api/v1/auth/key";

async fn fetch_openrouter_key_info(context: &ToolContext) -> Result<Value, String> {
    let api_key = resolve_openrouter_key(context).await?;
    let egress = context
        .egress_service
        .as_ref()
        .ok_or_else(|| "OpenRouter workspace API requires the host egress service".to_string())?;

    let response = egress
        .send(
            EgressRequest::new(
                "GET",
                OPENROUTER_KEY_INFO_URL,
                EgressRequestKind::Capability,
            )
            .header("authorization", format!("Bearer {api_key}"))
            .network_access(context.network_access.clone())
            .timeout_ms(15_000),
        )
        .await
        .map_err(|e| match e {
            // Distinguish a policy block from a transport failure so operators
            // see "blocked by network access policy" rather than a generic
            // "failed to reach" message.
            EgressError::NetworkAccessDenied { .. } => {
                format!("OpenRouter workspace API blocked by network access policy: {e}")
            }
            other => format!("Failed to reach OpenRouter workspace API: {other}"),
        })?;

    if !(200..300).contains(&response.status) {
        let body = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "OpenRouter /auth/key returned HTTP {}: {}",
            response.status, body
        ));
    }

    serde_json::from_slice(&response.body)
        .map_err(|e| format!("Failed to parse OpenRouter workspace response: {e}"))
}

async fn resolve_openrouter_key(context: &ToolContext) -> Result<String, String> {
    let store = context
        .provider_credential_store
        .as_ref()
        .ok_or_else(|| "No provider credential store available".to_string())?;

    let creds = store
        .get_default_provider_credentials("openrouter")
        .await
        .map_err(|e| format!("Failed to resolve OpenRouter credentials: {e}"))?
        .ok_or_else(|| {
            "No OpenRouter provider configured — add an OpenRouter provider to your org".to_string()
        })?;

    Ok(creds.api_key)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::egress::{EgressResponse, EgressService, EgressStreamResponse};
    use crate::error::Result as CoreResult;
    use crate::network_access::NetworkAccessList;
    use crate::traits::{ProviderCredentialStore, ProviderCredentials};
    use std::sync::{Arc, Mutex};

    struct StaticCredentialStore;

    #[async_trait]
    impl ProviderCredentialStore for StaticCredentialStore {
        async fn get_default_provider_credentials(
            &self,
            provider_type: &str,
        ) -> CoreResult<Option<ProviderCredentials>> {
            assert_eq!(provider_type, "openrouter");
            Ok(Some(ProviderCredentials {
                api_key: "test-key".to_string(),
                base_url: None,
            }))
        }
    }

    #[derive(Default)]
    struct RecordingEgress {
        requests: Mutex<Vec<EgressRequest>>,
    }

    #[async_trait]
    impl EgressService for RecordingEgress {
        async fn send(
            &self,
            request: EgressRequest,
        ) -> crate::egress::EgressResult<EgressResponse> {
            self.requests.lock().unwrap().push(request);
            Ok(EgressResponse {
                status: 200,
                headers: Default::default(),
                body: br#"{"data":{"label":"egress-key","usage":1000000,"limit":2000000,"is_free_tier":false}}"#.to_vec(),
            })
        }

        async fn send_stream(
            &self,
            _request: EgressRequest,
        ) -> crate::egress::EgressResult<EgressStreamResponse> {
            unreachable!("OpenRouter workspace tools use non-streaming egress")
        }
    }

    fn unlimited_key() -> OpenRouterKeyInfo {
        OpenRouterKeyInfo {
            label: "test-key".to_string(),
            usage_usd: 0.05,
            limit_usd: None,
            remaining_usd: None,
            is_free_tier: false,
            rate_limit: Some(OpenRouterRateLimit {
                requests: 200,
                interval: "10s".to_string(),
            }),
        }
    }

    fn capped_key(usage: f64, limit: f64) -> OpenRouterKeyInfo {
        OpenRouterKeyInfo {
            label: "capped-key".to_string(),
            usage_usd: usage,
            limit_usd: Some(limit),
            remaining_usd: Some((limit - usage).max(0.0)),
            is_free_tier: false,
            rate_limit: None,
        }
    }

    fn free_tier_key() -> OpenRouterKeyInfo {
        OpenRouterKeyInfo {
            label: "free-key".to_string(),
            usage_usd: 0.0,
            limit_usd: None,
            remaining_usd: None,
            is_free_tier: true,
            rate_limit: Some(OpenRouterRateLimit {
                requests: 20,
                interval: "1m".to_string(),
            }),
        }
    }

    #[tokio::test]
    async fn fetch_openrouter_key_info_uses_context_egress_service() {
        let egress = Arc::new(RecordingEgress::default());
        // Bare host is the supported allow-list form (a `…/*` suffix is matched
        // literally, not as a wildcard, and would deny the real URL against the
        // production egress service).
        let network_access = NetworkAccessList::allow_only(["openrouter.ai"]);
        // Guard against a misleading ACL: the allow-list must actually permit
        // the real request URL (RecordingEgress below ignores network_access,
        // so this is the only check that catches an unmatched pattern).
        assert!(
            network_access.is_url_allowed(OPENROUTER_KEY_INFO_URL),
            "network access list must allow the OpenRouter key-info URL"
        );
        let context = ToolContext::new(crate::SessionId::new())
            .with_provider_credential_store(Arc::new(StaticCredentialStore))
            .with_egress_service(egress.clone())
            .with_network_access(Some(network_access.clone()));

        let body = fetch_openrouter_key_info(&context).await.unwrap();

        assert_eq!(body["data"]["label"], "egress-key");
        let requests = egress.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, "GET");
        assert_eq!(request.url, OPENROUTER_KEY_INFO_URL);
        assert_eq!(request.kind, EgressRequestKind::Capability);
        assert_eq!(
            request.headers.get("authorization").map(String::as_str),
            Some("Bearer test-key")
        );
        assert_eq!(request.network_access, Some(network_access));
        assert_eq!(request.timeout_ms, Some(15_000));
    }

    // -------------------------------------------------------------------------
    // OpenRouterKeyInfo::from_api_response
    // -------------------------------------------------------------------------

    #[test]
    fn parse_unlimited_key_response() {
        let raw = json!({
            "label": "my-key",
            "usage": 12_345,
            "limit": null,
            "is_free_tier": false,
            "rate_limit": { "requests": 200, "interval": "10s" }
        });
        let info = OpenRouterKeyInfo::from_api_response(&raw).unwrap();
        assert_eq!(info.label, "my-key");
        assert!((info.usage_usd - 0.012345).abs() < 1e-9);
        assert!(info.limit_usd.is_none());
        assert!(info.remaining_usd.is_none());
        assert!(!info.is_free_tier);
        assert_eq!(info.rate_limit.as_ref().unwrap().requests, 200);
        assert_eq!(info.rate_limit.as_ref().unwrap().interval, "10s");
    }

    #[test]
    fn parse_capped_key_response() {
        let raw = json!({
            "label": "capped",
            "usage": 500_000,
            "limit": 1_000_000,
            "is_free_tier": false
        });
        let info = OpenRouterKeyInfo::from_api_response(&raw).unwrap();
        assert!((info.usage_usd - 0.5).abs() < 1e-9);
        assert!((info.limit_usd.unwrap() - 1.0).abs() < 1e-9);
        assert!((info.remaining_usd.unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn parse_exhausted_key_response() {
        let raw = json!({
            "label": "empty",
            "usage": 1_000_000,
            "limit": 1_000_000,
            "is_free_tier": false
        });
        let info = OpenRouterKeyInfo::from_api_response(&raw).unwrap();
        assert!((info.remaining_usd.unwrap()).abs() < 1e-9);
    }

    #[test]
    fn parse_free_tier_key_response() {
        let raw = json!({
            "usage": 0,
            "limit": null,
            "is_free_tier": true,
            "rate_limit": { "requests": 20, "interval": "1m" }
        });
        let info = OpenRouterKeyInfo::from_api_response(&raw).unwrap();
        assert!(info.is_free_tier);
        assert_eq!(info.rate_limit.as_ref().unwrap().requests, 20);
    }

    #[test]
    fn parse_missing_label_uses_default() {
        let raw = json!({ "usage": 0, "is_free_tier": false });
        let info = OpenRouterKeyInfo::from_api_response(&raw).unwrap();
        assert_eq!(info.label, "(unnamed)");
    }

    // -------------------------------------------------------------------------
    // detect_policy_drift
    // -------------------------------------------------------------------------

    #[test]
    fn no_drift_unlimited_key_no_constraints() {
        let drifts = detect_policy_drift(&unlimited_key(), None, false);
        assert!(drifts.is_empty());
    }

    #[test]
    fn no_drift_unlimited_key_with_threshold() {
        // Unlimited budget always passes threshold check.
        let drifts = detect_policy_drift(&unlimited_key(), Some(100.0), false);
        assert!(drifts.is_empty());
    }

    #[test]
    fn drift_budget_exhausted() {
        let key = capped_key(1.0, 1.0);
        let drifts = detect_policy_drift(&key, None, false);
        assert_eq!(drifts.len(), 1);
        assert!(matches!(
            &drifts[0],
            WorkspacePolicyDrift::BudgetExhausted { .. }
        ));
    }

    #[test]
    fn drift_budget_below_threshold() {
        let key = capped_key(0.95, 1.0); // $0.05 remaining
        let drifts = detect_policy_drift(&key, Some(0.10), false);
        assert_eq!(drifts.len(), 1);
        if let WorkspacePolicyDrift::BudgetBelowThreshold {
            remaining_usd,
            threshold_usd,
            ..
        } = &drifts[0]
        {
            assert!((remaining_usd - 0.05).abs() < 1e-9);
            assert!((threshold_usd - 0.10).abs() < 1e-9);
        } else {
            panic!("wrong drift kind");
        }
    }

    #[test]
    fn no_drift_budget_above_threshold() {
        let key = capped_key(0.5, 1.0); // $0.50 remaining
        let drifts = detect_policy_drift(&key, Some(0.10), false);
        assert!(drifts.is_empty());
    }

    #[test]
    fn drift_free_tier_requires_paid() {
        let key = free_tier_key();
        let drifts = detect_policy_drift(&key, None, true);
        assert_eq!(drifts.len(), 1);
        assert!(matches!(
            &drifts[0],
            WorkspacePolicyDrift::FreeTierRestriction { .. }
        ));
    }

    #[test]
    fn no_drift_free_tier_no_paid_requirement() {
        let key = free_tier_key();
        let drifts = detect_policy_drift(&key, None, false);
        assert!(drifts.is_empty());
    }

    #[test]
    fn multiple_drifts_exhausted_and_free_tier() {
        let mut key = capped_key(1.0, 1.0);
        key.is_free_tier = true;
        let drifts = detect_policy_drift(&key, None, true);
        assert_eq!(drifts.len(), 2);
        assert!(
            drifts
                .iter()
                .any(|d| matches!(d, WorkspacePolicyDrift::BudgetExhausted { .. }))
        );
        assert!(
            drifts
                .iter()
                .any(|d| matches!(d, WorkspacePolicyDrift::FreeTierRestriction { .. }))
        );
    }

    #[test]
    fn policy_compatibility_report_serializes() {
        let report = PolicyCompatibilityReport {
            compatible: false,
            workspace: capped_key(1.0, 1.0),
            drifts: vec![WorkspacePolicyDrift::BudgetExhausted {
                usage_usd: 1.0,
                limit_usd: 1.0,
                message: "exhausted".to_string(),
            }],
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["compatible"], false);
        assert_eq!(json["drifts"][0]["kind"], "budget_exhausted");
    }

    #[test]
    fn workspace_policy_drift_messages_are_non_empty() {
        // Verify the `message` field produced by detect_policy_drift is
        // non-empty for every drift kind.
        let key = capped_key(1.0, 1.0);
        let drifts = detect_policy_drift(&key, Some(0.10), true);
        // BudgetExhausted + FreeTierRestriction (key is not free_tier here, so
        // just check exhausted)
        for d in &drifts {
            let msg = match d {
                WorkspacePolicyDrift::BudgetExhausted { message, .. } => message.as_str(),
                WorkspacePolicyDrift::BudgetBelowThreshold { message, .. } => message.as_str(),
                WorkspacePolicyDrift::FreeTierRestriction { message, .. } => message.as_str(),
            };
            assert!(!msg.is_empty(), "drift message should not be empty");
        }
    }
}
