//! OpenRouter per-call options: routing controls, server tools, plugins, attribution.
//!
//! These types version with the OpenRouter API, so they live in the OpenRouter
//! driver crate — not in `everruns-provider`. The abstract provider crate only
//! carries them opaquely: routing travels in `LlmCallConfig::driver_options`
//! under [`OPENROUTER_ROUTING_OPTION_KEY`], and attribution travels in
//! `LlmCallConfig::metadata` under the keys below. Nothing outside this crate
//! should duplicate these shapes.

use std::collections::HashMap;

/// `driver_options` key under which producers stash an [`OpenRouterRoutingConfig`]
/// (as JSON) for this driver to consume.
pub const OPENROUTER_ROUTING_OPTION_KEY: &str = "openrouter/routing";

/// Stash a routing config in a `driver_options` map under
/// [`OPENROUTER_ROUTING_OPTION_KEY`]. Serialization of this struct is
/// infallible in practice; a failure stores `Value::Null`, which the driver
/// rejects with a clear invalid-request error instead of sending it.
pub fn insert_routing_option(
    options: &mut HashMap<String, serde_json::Value>,
    routing: &OpenRouterRoutingConfig,
) {
    let value = serde_json::to_value(routing).unwrap_or(serde_json::Value::Null);
    options.insert(OPENROUTER_ROUTING_OPTION_KEY.to_string(), value);
}

/// True when the stashed routing option requests provider-executed
/// `server_tools`. Retry policy and other driver-external callers use this
/// instead of depending on the routing shape.
pub fn has_server_tools(options: &HashMap<String, serde_json::Value>) -> bool {
    options
        .get(OPENROUTER_ROUTING_OPTION_KEY)
        .and_then(|raw| serde_json::from_value::<OpenRouterRoutingConfig>(raw.clone()).ok())
        .is_some_and(|routing| !routing.server_tools.is_empty())
}

/// High-level intent presets that compile into OpenRouter provider-routing
/// controls. Presets let callers express quality, cost, privacy, and capability
/// goals without knowing every OpenRouter `provider` flag.
///
/// Multiple presets may be combined. When a preset and an explicit `provider`
/// field target the same control, the explicit field wins. Presets applied
/// earlier in the list may be overridden by later ones for the same field.
///
/// Compilation happens in `OpenRouterRoutingConfig::apply_presets()`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OpenRouterRoutingPreset {
    /// Prefer the cheapest providers that support function-calling parameters.
    CheapestWithTools,
    /// Prefer the highest-throughput providers for quick review or triage tasks.
    LowestLatencyReview,
    /// Route only to zero-data-retention (ZDR) endpoints.
    ZdrOnly,
    /// Try BYOK-registered providers first; fall back to shared capacity.
    ByokFirst,
    /// Deny all provider-side data collection (logs and training).
    NoDataCollection,
    /// Route only to providers that support strict JSON / structured output.
    StrictJson,
    /// Route only to providers that natively support reasoning/thinking models.
    ReasoningRequired,
    /// Cap per-token provider cost. Values are USD per million tokens; `None`
    /// means no cap on that dimension.
    MaxPrice {
        /// Maximum prompt cost in USD per million tokens.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt_usd_per_million: Option<f64>,
        /// Maximum completion cost in USD per million tokens.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        completion_usd_per_million: Option<f64>,
    },
}

/// OpenRouter model fallback and provider routing controls.
///
/// Organization-level strategy for how OpenRouter should allocate compute capacity.
///
/// Controls whether requests use OpenRouter shared credits, prefer customer-owned
/// upstream keys (BYOK), or require BYOK-only routing. Compiled into OpenRouter
/// `provider` routing controls before dispatch; not sent verbatim on the wire.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenRouterCapacityStrategy {
    /// Use OpenRouter shared capacity (credits). No routing changes. Default.
    #[default]
    SharedCapacity,
    /// Prefer providers where the org has registered its own upstream key.
    /// Falls back to shared capacity when BYOK providers are unavailable.
    /// Sets `provider.allow_fallbacks = true` unless the caller overrides it.
    ByokFirst,
    /// Require a provider where the org has its own upstream key.
    /// Routing fails if `provider.only` is not explicitly configured with at
    /// least one BYOK provider slug.
    /// Sets `provider.allow_fallbacks = false`.
    ByokOnly,
}

/// One of OpenRouter's provider-executed "server tools" (beta).
///
/// Server tools are tools OpenRouter runs server-side — it loops internally and
/// returns the final answer, so unlike client-executed function tools the agent
/// loop never dispatches them. The only client-visible artifact is
/// `usage.server_tool_use`. See
/// <https://openrouter.ai/docs/guides/features/server-tools>.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenRouterServerToolKind {
    WebSearch,
    WebFetch,
    Datetime,
    ImageGeneration,
    ApplyPatch,
    Fusion,
    Advisor,
    Subagent,
}

impl OpenRouterServerToolKind {
    /// Every known server tool, in catalog order.
    pub const ALL: [OpenRouterServerToolKind; 8] = [
        Self::WebSearch,
        Self::WebFetch,
        Self::Datetime,
        Self::ImageGeneration,
        Self::ApplyPatch,
        Self::Fusion,
        Self::Advisor,
        Self::Subagent,
    ];

    /// Bare tool name (no prefix), e.g. `"web_search"`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::WebSearch => "web_search",
            Self::WebFetch => "web_fetch",
            Self::Datetime => "datetime",
            Self::ImageGeneration => "image_generation",
            Self::ApplyPatch => "apply_patch",
            Self::Fusion => "fusion",
            Self::Advisor => "advisor",
            Self::Subagent => "subagent",
        }
    }

    /// Human-readable English display name, used for UI schema titles.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::WebSearch => "Web Search",
            Self::WebFetch => "Web Fetch",
            Self::Datetime => "Date & Time",
            Self::ImageGeneration => "Image Generation",
            Self::ApplyPatch => "Apply Patch",
            Self::Fusion => "Fusion",
            Self::Advisor => "Advisor",
            Self::Subagent => "Subagent",
        }
    }

    /// The `type` discriminator OpenRouter expects in the request `tools` array,
    /// e.g. `"openrouter:web_search"`.
    pub fn wire_type(&self) -> String {
        format!("openrouter:{}", self.name())
    }

    /// Parse a bare tool name (no `openrouter:` prefix).
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

/// One activated OpenRouter server tool plus optional tool-specific parameters
/// (e.g. web_search `max_results`). Parameters are forwarded verbatim under the
/// wire entry's `parameters` field.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OpenRouterServerTool {
    pub kind: OpenRouterServerToolKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
}

impl OpenRouterServerTool {
    /// A server tool with no parameters.
    pub fn new(kind: OpenRouterServerToolKind) -> Self {
        Self {
            kind,
            parameters: None,
        }
    }

    /// A server tool carrying parameters forwarded verbatim to OpenRouter.
    pub fn with_parameters(kind: OpenRouterServerToolKind, parameters: serde_json::Value) -> Self {
        Self {
            kind,
            parameters: Some(parameters),
        }
    }
}

/// These fields mirror OpenRouter's request-level routing extensions. Drivers
/// must only forward this config to OpenRouter-compatible endpoints.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OpenRouterRoutingConfig {
    /// Candidate models to try in OpenRouter's fallback order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    /// OpenRouter route strategy. Currently `fallback` is the stable route
    /// value used with `models`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<OpenRouterRoute>,
    /// Provider ordering, policy, and sorting preferences.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<OpenRouterProviderRouting>,
    /// Optional plugin activations (web search, file reader).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugins: Option<OpenRouterPluginConfig>,
    /// Org-level capacity strategy. Compiled into `provider` routing before
    /// dispatch; not forwarded verbatim. `None` and `SharedCapacity` are
    /// equivalent (no routing changes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity_strategy: Option<OpenRouterCapacityStrategy>,
    /// High-level routing quality/policy presets. Compiled into `provider`
    /// flags by `apply_presets()` before the request is serialized.
    /// Explicit `provider` fields override preset-derived values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub presets: Vec<OpenRouterRoutingPreset>,
    /// OpenRouter server tools (beta) the model may invoke. Provider-executed;
    /// appended to the request `tools` array as `{"type":"openrouter:<name>"}`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub server_tools: Vec<OpenRouterServerTool>,
}

impl OpenRouterRoutingConfig {
    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
            && self.route.is_none()
            && self.provider.is_none()
            && self.plugins.as_ref().is_none_or(|p| p.is_empty())
            && matches!(
                self.capacity_strategy,
                None | Some(OpenRouterCapacityStrategy::SharedCapacity)
            )
            && self.presets.is_empty()
            && self.server_tools.is_empty()
    }

    /// Build an ordered model-fallback routing config.
    pub fn fallback_models(models: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let models = models.into_iter().map(Into::into).collect::<Vec<_>>();
        let route = (!models.is_empty()).then_some(OpenRouterRoute::Fallback);
        Self {
            models,
            route,
            provider: None,
            plugins: None,
            capacity_strategy: None,
            presets: vec![],
            server_tools: vec![],
        }
    }

    pub fn validate_for_primary_model(
        &self,
        primary_model: &str,
    ) -> std::result::Result<(), String> {
        if self.route == Some(OpenRouterRoute::Fallback) && self.models.is_empty() {
            return Err(
                "OpenRouter fallback routing requires at least one model in `models`".to_string(),
            );
        }

        if let Some(first_model) = self.models.first()
            && first_model != primary_model
        {
            return Err(format!(
                "OpenRouter routing models[0] ('{first_model}') must match primary model ('{primary_model}')"
            ));
        }

        Ok(())
    }

    /// Apply the capacity strategy, returning a derived config with `provider`
    /// routing adjusted accordingly.
    ///
    /// - `SharedCapacity` / `None` — returns `self` unchanged.
    /// - `ByokFirst` — sets `provider.allow_fallbacks = true` when not already set.
    /// - `ByokOnly` — requires `provider.only` to list at least one provider slug;
    ///   sets `provider.allow_fallbacks = false`.
    ///
    /// Returns `Err` when the strategy constraints cannot be satisfied.
    pub fn apply_capacity_strategy(&self) -> std::result::Result<Self, String> {
        match self.capacity_strategy {
            None | Some(OpenRouterCapacityStrategy::SharedCapacity) => Ok(self.clone()),
            Some(OpenRouterCapacityStrategy::ByokFirst) => {
                let mut result = self.clone();
                let provider = result.provider.get_or_insert_with(Default::default);
                if provider.allow_fallbacks.is_none() {
                    provider.allow_fallbacks = Some(true);
                }
                Ok(result)
            }
            Some(OpenRouterCapacityStrategy::ByokOnly) => {
                let only_is_empty = self.provider.as_ref().is_none_or(|p| p.only.is_empty());
                if only_is_empty {
                    return Err(
                        "OpenRouter BYOK-only strategy requires provider.only to list at least \
                         one upstream provider slug. Configure the provider list to match the \
                         BYOK providers registered in your OpenRouter workspace."
                            .to_string(),
                    );
                }
                let mut result = self.clone();
                let provider = result.provider.get_or_insert_with(Default::default);
                provider.allow_fallbacks = Some(false);
                Ok(result)
            }
        }
    }

    /// Compile `presets` into `OpenRouterProviderRouting` flags and merge with
    /// any explicit `provider` overrides. Returns a derived config with the
    /// `presets` list cleared and `provider` reflecting the merged result.
    ///
    /// Explicit `provider` fields always win over preset-derived values. When
    /// multiple presets target the same provider field, later presets in the
    /// list override earlier ones.
    ///
    /// Returns `Err` if any preset values are invalid (e.g. negative `MaxPrice` values).
    pub fn apply_presets(&self) -> std::result::Result<Self, String> {
        if self.presets.is_empty() {
            return Ok(self.clone());
        }

        let mut derived = OpenRouterProviderRouting::default();

        for preset in &self.presets {
            match preset {
                OpenRouterRoutingPreset::CheapestWithTools => {
                    derived.require_parameters = Some(true);
                    derived.sort = Some(OpenRouterProviderSort::Simple(
                        OpenRouterProviderSortBy::Price,
                    ));
                }
                OpenRouterRoutingPreset::LowestLatencyReview => {
                    derived.sort = Some(OpenRouterProviderSort::Simple(
                        OpenRouterProviderSortBy::Throughput,
                    ));
                }
                OpenRouterRoutingPreset::ZdrOnly => {
                    derived.zdr = Some(true);
                }
                OpenRouterRoutingPreset::ByokFirst => {
                    if derived.allow_fallbacks.is_none() {
                        derived.allow_fallbacks = Some(true);
                    }
                }
                OpenRouterRoutingPreset::NoDataCollection => {
                    derived.data_collection = Some(OpenRouterDataCollection::Deny);
                }
                OpenRouterRoutingPreset::StrictJson
                | OpenRouterRoutingPreset::ReasoningRequired => {
                    derived.require_parameters = Some(true);
                }
                OpenRouterRoutingPreset::MaxPrice {
                    prompt_usd_per_million,
                    completion_usd_per_million,
                } => {
                    if prompt_usd_per_million.is_some_and(|v| v < 0.0)
                        || completion_usd_per_million.is_some_and(|v| v < 0.0)
                    {
                        return Err(
                            "MaxPrice preset values must be non-negative USD per million tokens"
                                .to_string(),
                        );
                    }
                    if prompt_usd_per_million.is_some() || completion_usd_per_million.is_some() {
                        let mp = derived.max_price.get_or_insert_with(Default::default);
                        if let Some(p) = prompt_usd_per_million {
                            // Routing ceilings use USD per million, unlike model catalog pricing.
                            mp.prompt = Some(*p);
                        }
                        if let Some(c) = completion_usd_per_million {
                            mp.completion = Some(*c);
                        }
                    }
                }
            }
        }

        // Explicit provider fields override preset-derived values.
        let merged = merge_provider_routing(derived, self.provider.clone().unwrap_or_default());

        let mut result = self.clone();
        result.presets = vec![];
        result.provider = if merged.is_empty() {
            None
        } else {
            Some(merged)
        };
        Ok(result)
    }
}

/// Merge preset-derived provider routing with explicit provider overrides.
/// Explicit fields always win; preset-derived fields fill gaps where explicit
/// fields are absent (None / empty Vec).
fn merge_provider_routing(
    derived: OpenRouterProviderRouting,
    explicit: OpenRouterProviderRouting,
) -> OpenRouterProviderRouting {
    OpenRouterProviderRouting {
        order: if !explicit.order.is_empty() {
            explicit.order
        } else {
            derived.order
        },
        only: if !explicit.only.is_empty() {
            explicit.only
        } else {
            derived.only
        },
        ignore: if !explicit.ignore.is_empty() {
            explicit.ignore
        } else {
            derived.ignore
        },
        allow_fallbacks: explicit.allow_fallbacks.or(derived.allow_fallbacks),
        require_parameters: explicit.require_parameters.or(derived.require_parameters),
        data_collection: explicit.data_collection.or(derived.data_collection),
        zdr: explicit.zdr.or(derived.zdr),
        enforce_distillable_text: explicit
            .enforce_distillable_text
            .or(derived.enforce_distillable_text),
        quantizations: if !explicit.quantizations.is_empty() {
            explicit.quantizations
        } else {
            derived.quantizations
        },
        sort: explicit.sort.or(derived.sort),
        max_price: explicit.max_price.or(derived.max_price),
    }
}

/// OpenRouter route strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenRouterRoute {
    Fallback,
}

/// OpenRouter provider routing preferences.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OpenRouterProviderRouting {
    /// Provider slugs to try first, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<String>,
    /// Restrict routing to these provider slugs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub only: Vec<String>,
    /// Provider slugs to skip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignore: Vec<String>,
    /// Whether OpenRouter may fall back outside the ordered/allowed providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_fallbacks: Option<bool>,
    /// Require routed providers to support all request parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_parameters: Option<bool>,
    /// Restrict routing by provider data-retention policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_collection: Option<OpenRouterDataCollection>,
    /// Restrict routing to zero-data-retention endpoints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zdr: Option<bool>,
    /// Restrict routing to distillable-text endpoints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforce_distillable_text: Option<bool>,
    /// Restrict routing to provider quantization levels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quantizations: Vec<String>,
    /// Sort provider endpoints by price, throughput, or latency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<OpenRouterProviderSort>,
    /// Maximum accepted per-unit provider price.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_price: Option<OpenRouterMaxPrice>,
}

impl OpenRouterProviderRouting {
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
            && self.only.is_empty()
            && self.ignore.is_empty()
            && self.allow_fallbacks.is_none()
            && self.require_parameters.is_none()
            && self.data_collection.is_none()
            && self.zdr.is_none()
            && self.enforce_distillable_text.is_none()
            && self.quantizations.is_empty()
            && self.sort.is_none()
            && self.max_price.is_none()
    }
}

/// OpenRouter provider data-retention preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenRouterDataCollection {
    Allow,
    Deny,
}

/// OpenRouter provider sort preference.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum OpenRouterProviderSort {
    Simple(OpenRouterProviderSortBy),
    Advanced(OpenRouterProviderSortOptions),
}

/// OpenRouter provider sorting dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenRouterProviderSortBy {
    Price,
    Throughput,
    Latency,
}

/// OpenRouter advanced provider sort options.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OpenRouterProviderSortOptions {
    pub by: OpenRouterProviderSortBy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partition: Option<OpenRouterSortPartition>,
}

/// How OpenRouter sorts endpoints when multiple fallback models are present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenRouterSortPartition {
    Model,
    None,
}

/// Maximum accepted OpenRouter provider pricing, expressed in dollars per
/// million prompt/completion tokens or per request/image where supported.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OpenRouterMaxPrice {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<f64>,
}

/// OpenRouter web-search plugin configuration.
///
/// Instructs OpenRouter to retrieve and inject web search results before the
/// model sees the prompt. Only sent when the resolved provider type is
/// OpenRouter.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OpenRouterWebSearchPlugin {
    /// Maximum number of search results to include.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
    /// Custom search prompt hint passed to the web-search step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_prompt: Option<String>,
}

/// OpenRouter file-reader plugin configuration.
///
/// Instructs OpenRouter to read and attach file contents before the model
/// sees the prompt. Only sent when the resolved provider type is OpenRouter.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OpenRouterFilePlugin {}

/// OpenRouter plugin configuration bundling optional plugin activations.
///
/// Any `None` plugin is omitted from the wire request. When all plugins are
/// `None`, no `plugins` field is emitted.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OpenRouterPluginConfig {
    /// Web-search plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<OpenRouterWebSearchPlugin>,
    /// File-reader plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<OpenRouterFilePlugin>,
}

impl OpenRouterPluginConfig {
    pub fn is_empty(&self) -> bool {
        self.web.is_none() && self.file.is_none()
    }
}

/// Metadata key consumed by the OpenRouter driver as `HTTP-Referer`.
pub const OPENROUTER_HTTP_REFERER_METADATA_KEY: &str = "openrouter.http_referer";
/// Metadata key consumed by the OpenRouter driver as `X-Title`.
pub const OPENROUTER_X_TITLE_METADATA_KEY: &str = "openrouter.x_title";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openrouter_routing_serializes_request_fields() {
        let routing = OpenRouterRoutingConfig {
            models: vec![
                "openai/gpt-5-mini".to_string(),
                "anthropic/claude-sonnet-4.5".to_string(),
            ],
            route: Some(OpenRouterRoute::Fallback),
            provider: Some(OpenRouterProviderRouting {
                order: vec!["anthropic".to_string(), "openai".to_string()],
                allow_fallbacks: Some(false),
                require_parameters: Some(true),
                data_collection: Some(OpenRouterDataCollection::Deny),
                zdr: Some(true),
                sort: Some(OpenRouterProviderSort::Advanced(
                    OpenRouterProviderSortOptions {
                        by: OpenRouterProviderSortBy::Throughput,
                        partition: Some(OpenRouterSortPartition::None),
                    },
                )),
                max_price: Some(OpenRouterMaxPrice {
                    prompt: Some(1.0),
                    completion: Some(2.0),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let json = serde_json::to_value(routing).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "models": [
                    "openai/gpt-5-mini",
                    "anthropic/claude-sonnet-4.5"
                ],
                "route": "fallback",
                "provider": {
                    "order": ["anthropic", "openai"],
                    "allow_fallbacks": false,
                    "require_parameters": true,
                    "data_collection": "deny",
                    "zdr": true,
                    "sort": {
                        "by": "throughput",
                        "partition": "none"
                    },
                    "max_price": {
                        "prompt": 1.0,
                        "completion": 2.0
                    }
                }
            })
        );
    }

    #[test]
    fn fallback_routing_preserves_order_and_rejects_invalid_primary() {
        let empty = OpenRouterRoutingConfig::fallback_models(std::iter::empty::<String>());
        assert_eq!(serde_json::to_value(&empty).unwrap(), serde_json::json!({}));
        assert!(empty.is_empty());
        assert_eq!(empty.validate_for_primary_model("primary"), Ok(()));
        let routing = OpenRouterRoutingConfig::fallback_models(["primary", "backup", "primary"]);
        assert_eq!(
            serde_json::to_value(&routing).unwrap(),
            serde_json::json!({"models":["primary","backup","primary"],"route":"fallback"})
        );
        assert_eq!(routing.validate_for_primary_model("primary"), Ok(()));
        assert_eq!(
            routing.validate_for_primary_model("backup").unwrap_err(),
            "OpenRouter routing models[0] ('primary') must match primary model ('backup')"
        );
        assert_eq!(
            OpenRouterRoutingConfig {
                route: Some(OpenRouterRoute::Fallback),
                ..Default::default()
            }
            .validate_for_primary_model("primary")
            .unwrap_err(),
            "OpenRouter fallback routing requires at least one model in `models`"
        );
    }

    #[test]
    fn routing_emptiness_preserves_each_actionable_control_in_call_config() {
        use serde_json::json;
        for (wire, active) in [
            (json!({}), false),
            (json!({"plugins":{}}), false),
            (json!({"capacity_strategy":"shared_capacity"}), false),
            (json!({"models":["model"]}), true),
            (json!({"route":"fallback"}), true),
            (json!({"provider":{"allow_fallbacks":false}}), true),
            (json!({"plugins":{"web":{}}}), true),
            (json!({"plugins":{"file":{}}}), true),
            (json!({"capacity_strategy":"byok_first"}), true),
            (json!({"capacity_strategy":"byok_only"}), true),
            (json!({"presets":[{"kind":"zdr_only"}]}), true),
        ] {
            let routing: OpenRouterRoutingConfig = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(!routing.is_empty(), active, "{wire}");
        }
    }

    #[test]
    fn plugin_payloads_preserve_options_and_omit_absent_fields() {
        use serde_json::json;
        for wire in [
            json!({}),
            json!({"web":{}}),
            json!({"file":{}}),
            json!({"web":{"max_results":10,"search_prompt":"search for Rust crates"},"file":{}}),
        ] {
            let plugins: OpenRouterPluginConfig = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(plugins.is_empty(), wire == json!({}));
            assert_eq!(serde_json::to_value(plugins).unwrap(), wire);
        }
    }

    #[test]
    fn capacity_strategies_preserve_unrelated_routing_and_enforce_byok_policy() {
        use serde_json::json;
        for (strategy, explicit, expected) in [
            (None, None, None),
            (Some("shared_capacity"), Some(false), Some(false)),
            (Some("byok_first"), None, Some(true)),
            (Some("byok_first"), Some(false), Some(false)),
            (Some("byok_first"), Some(true), Some(true)),
            (Some("byok_only"), Some(true), Some(false)),
        ] {
            let mut wire = json!({"models":["primary","backup"],"provider":{"only":["my-byok-provider"],"order":["first"],"zdr":true},"plugins":{"file":{}},"presets":[{"kind":"strict_json"}]});
            if let Some(strategy) = strategy {
                wire["capacity_strategy"] = json!(strategy);
            }
            if let Some(value) = explicit {
                wire["provider"]["allow_fallbacks"] = json!(value);
            }
            let base: OpenRouterRoutingConfig = serde_json::from_value(wire.clone()).unwrap();
            let result = base.apply_capacity_strategy().unwrap();
            let mut expected_wire = wire.clone();
            if let Some(value) = expected {
                expected_wire["provider"]["allow_fallbacks"] = json!(value);
            }
            assert_eq!(serde_json::to_value(&result).unwrap(), expected_wire);
            assert_eq!(serde_json::to_value(&base).unwrap(), wire);
            assert_eq!(result.apply_capacity_strategy().unwrap(), result);
        }
        for provider in [None, Some(OpenRouterProviderRouting::default())] {
            let base = OpenRouterRoutingConfig {
                capacity_strategy: Some(OpenRouterCapacityStrategy::ByokOnly),
                provider,
                ..Default::default()
            };
            assert_eq!(
                base.apply_capacity_strategy().unwrap_err(),
                "OpenRouter BYOK-only strategy requires provider.only to list at least one upstream provider slug. Configure the provider list to match the BYOK providers registered in your OpenRouter workspace."
            );
        }
    }

    #[test]
    fn presets_compile_to_complete_literal_routing_and_clear_once() {
        use serde_json::json;
        for (preset, provider) in [
            (
                OpenRouterRoutingPreset::CheapestWithTools,
                json!({"require_parameters":true,"sort":"price"}),
            ),
            (
                OpenRouterRoutingPreset::LowestLatencyReview,
                json!({"sort":"throughput"}),
            ),
            (OpenRouterRoutingPreset::ZdrOnly, json!({"zdr":true})),
            (
                OpenRouterRoutingPreset::ByokFirst,
                json!({"allow_fallbacks":true}),
            ),
            (
                OpenRouterRoutingPreset::NoDataCollection,
                json!({"data_collection":"deny"}),
            ),
            (
                OpenRouterRoutingPreset::StrictJson,
                json!({"require_parameters":true}),
            ),
            (
                OpenRouterRoutingPreset::ReasoningRequired,
                json!({"require_parameters":true}),
            ),
        ] {
            let base = OpenRouterRoutingConfig {
                models: vec!["primary".into()],
                presets: vec![preset],
                ..Default::default()
            };
            let before = base.clone();
            let result = base.apply_presets().unwrap();
            assert_eq!(
                serde_json::to_value(&result).unwrap(),
                json!({"models":["primary"],"provider":provider})
            );
            assert_eq!(base, before);
            assert_eq!(result.apply_presets().unwrap(), result);
        }
        let base = OpenRouterRoutingConfig::fallback_models(["primary", "backup"]);
        assert_eq!(base.apply_presets().unwrap(), base);
    }

    #[test]
    fn preset_precedence_preserves_explicit_false_values_and_every_provider_field() {
        use serde_json::json;
        let presets = vec![
            OpenRouterRoutingPreset::CheapestWithTools,
            OpenRouterRoutingPreset::ZdrOnly,
            OpenRouterRoutingPreset::NoDataCollection,
            OpenRouterRoutingPreset::ByokFirst,
            OpenRouterRoutingPreset::LowestLatencyReview,
        ];
        let combined = OpenRouterRoutingConfig {
            presets: presets.clone(),
            ..Default::default()
        }
        .apply_presets()
        .unwrap();
        assert_eq!(
            serde_json::to_value(&combined).unwrap(),
            json!({"provider":{"require_parameters":true,"sort":"throughput","zdr":true,"data_collection":"deny","allow_fallbacks":true}})
        );
        let explicit = json!({"order":["second","first"],"only":["allowed"],"ignore":["ignored"],"allow_fallbacks":false,"require_parameters":false,"data_collection":"allow","zdr":false,"enforce_distillable_text":false,"quantizations":["fp8"],"sort":{"by":"latency","partition":"model"},"max_price":{"prompt":7.0,"completion":8.0,"request":0.2,"image":0.1}});
        let base = OpenRouterRoutingConfig {
            presets,
            provider: Some(serde_json::from_value(explicit.clone()).unwrap()),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(base.apply_presets().unwrap()).unwrap(),
            json!({"provider":explicit})
        );
        let partial = OpenRouterRoutingConfig {
            presets: vec![OpenRouterRoutingPreset::CheapestWithTools],
            provider: Some(OpenRouterProviderRouting {
                sort: Some(OpenRouterProviderSort::Simple(
                    OpenRouterProviderSortBy::Throughput,
                )),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(partial.apply_presets().unwrap()).unwrap(),
            json!({"provider":{"sort":"throughput","require_parameters":true}})
        );
    }

    #[test]
    fn price_presets_preserve_literal_usd_per_million_and_dimension_boundaries() {
        use serde_json::json;
        for (prompt, completion, expected) in [
            (
                Some(5.0),
                Some(15.0),
                json!({"provider":{"max_price":{"prompt":5.0,"completion":15.0}}}),
            ),
            (
                Some(0.0),
                None,
                json!({"provider":{"max_price":{"prompt":0.0}}}),
            ),
            (
                None,
                Some(0.5),
                json!({"provider":{"max_price":{"completion":0.5}}}),
            ),
            (None, None, json!({})),
        ] {
            let base = OpenRouterRoutingConfig {
                presets: vec![OpenRouterRoutingPreset::MaxPrice {
                    prompt_usd_per_million: prompt,
                    completion_usd_per_million: completion,
                }],
                ..Default::default()
            };
            assert_eq!(
                serde_json::to_value(base.apply_presets().unwrap()).unwrap(),
                expected
            );
        }
        for (prompt, completion) in [(Some(-1.0), None), (None, Some(-0.01))] {
            let base = OpenRouterRoutingConfig {
                presets: vec![OpenRouterRoutingPreset::MaxPrice {
                    prompt_usd_per_million: prompt,
                    completion_usd_per_million: completion,
                }],
                ..Default::default()
            };
            assert_eq!(
                base.apply_presets().unwrap_err(),
                "MaxPrice preset values must be non-negative USD per million tokens"
            );
        }
    }
}
