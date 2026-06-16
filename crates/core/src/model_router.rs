// Model Router domain types
//
// Design intent lives in `specs/model-router.md`.
//
// A Model Router is an org-scoped, named container of named routes. Each
// route picks a concrete LLM model via a strategy and a list of candidates;
// candidates carry provider-agnostic request overrides (reasoning_effort,
// temperature, etc.). Harnesses, agents, sessions, and org settings can bind
// to either a concrete model (today's behavior, preserved) or to a router.
//
// This module defines the entity, the route, the candidate, the strategy
// enum, and structural validation. Storage trait, REST APIs, runtime
// resolver, binding migrations, and UI ship as follow-up vertical slices.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::driver_registry::OpenRouterRoutingConfig;
use crate::typed_id::{ModelId, ModelRouterId};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Router lifecycle status. Mirrors other building-block lifecycles.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ModelRouterStatus {
    Active,
    Archived,
    Deleted,
}

impl std::fmt::Display for ModelRouterStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelRouterStatus::Active => write!(f, "active"),
            ModelRouterStatus::Archived => write!(f, "archived"),
            ModelRouterStatus::Deleted => write!(f, "deleted"),
        }
    }
}

impl From<&str> for ModelRouterStatus {
    fn from(s: &str) -> Self {
        match s {
            "archived" => ModelRouterStatus::Archived,
            "deleted" => ModelRouterStatus::Deleted,
            _ => ModelRouterStatus::Active,
        }
    }
}

/// Selection strategy for a route. See `specs/model-router.md` for behavior.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ModelRouterStrategy {
    /// Exactly one candidate; trivial selection.
    Single,
    /// Try candidates in `position` order; fall through to the next on
    /// transient errors.
    OrderedFallback,
    /// Sample a candidate by `weight` per call.
    Weighted,
    /// Evaluate candidate `rules` against binding `params`; first match wins.
    Rules,
    /// Hand off to an embedded resolver registered by the host runtime.
    /// Database stores the candidate list as advisory metadata.
    Custom,
}

impl std::fmt::Display for ModelRouterStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelRouterStrategy::Single => write!(f, "single"),
            ModelRouterStrategy::OrderedFallback => write!(f, "ordered_fallback"),
            ModelRouterStrategy::Weighted => write!(f, "weighted"),
            ModelRouterStrategy::Rules => write!(f, "rules"),
            ModelRouterStrategy::Custom => write!(f, "custom"),
        }
    }
}

impl ModelRouterStrategy {
    /// Parse from the canonical string form (matches the DB CHECK constraint
    /// on `model_router_routes.strategy`).
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "single" => Ok(ModelRouterStrategy::Single),
            "ordered_fallback" => Ok(ModelRouterStrategy::OrderedFallback),
            "weighted" => Ok(ModelRouterStrategy::Weighted),
            "rules" => Ok(ModelRouterStrategy::Rules),
            "custom" => Ok(ModelRouterStrategy::Custom),
            other => Err(format!(
                "unknown model router strategy '{other}'; expected one of single, ordered_fallback, weighted, rules, custom"
            )),
        }
    }
}

/// A Model Router — org-scoped named container of routes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ModelRouter {
    /// External identifier (`mrtr_<32-hex>`). Shown as `id` in API responses.
    #[serde(rename = "id")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, example = "mrtr_01933b5a000070008000000000000001")
    )]
    pub public_id: ModelRouterId,
    /// Internal UUID primary key. Used for FK references. Never exposed in API.
    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,
    /// Human-readable name, unique per org while not deleted.
    pub name: String,
    /// Optional description for the UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON Schema describing caller-supplied params validated at binding
    /// time. Defaults to an empty object — `{}` means no parameters expected
    /// — never JSON `null`, so wire and DB shapes stay consistent.
    #[serde(default = "default_empty_object")]
    pub param_schema: serde_json::Value,
    pub status: ModelRouterStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
    /// Routes belonging to this router. Populated by service-layer joins.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<ModelRouterRoute>,
}

/// A named route inside a router. Carries the human-facing `purpose` and
/// the model-facing `when_to_use` description used by the future
/// `set_model` discoverability tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ModelRouterRoute {
    pub id: Uuid,
    /// Stable identifier within router (e.g. `base`, `analysis`).
    pub key: String,
    /// Human-facing label.
    pub purpose: String,
    /// Model-facing description (used by future `set_model` tool).
    pub when_to_use: String,
    pub strategy: ModelRouterStrategy,
    /// Display order within router.
    pub position: i32,
    /// Candidates inside this route, in position order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<ModelRouterCandidate>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// An ordered candidate inside a route. References a concrete model and may
/// carry provider-agnostic request overrides plus a weight (for `weighted`)
/// or rules (for `rules`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ModelRouterCandidate {
    pub id: Uuid,
    /// The concrete model to invoke.
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, example = "model_01933b5a000070008000000000000001")
    )]
    pub model_id: ModelId,
    /// Provider-agnostic overrides applied at LLM-call time
    /// (`reasoning_effort`, `temperature`, `max_output_tokens`, ...).
    /// Defaults to an empty object so missing fields stay object-shaped on
    /// the wire, matching the DB JSONB default of `{}`.
    #[serde(default = "default_empty_object")]
    pub request_overrides: serde_json::Value,
    /// Used by `weighted` strategy. Defaults to `1` (uniform).
    #[serde(default = "default_weight")]
    pub weight: i32,
    /// Used by `rules` strategy. None for other strategies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<serde_json::Value>,
    /// Used by `ordered_fallback` strategy.
    pub position: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn default_weight() -> i32 {
    1
}

fn default_empty_object() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

/// Maximum length for a route key (matches the DB column).
pub const MAX_ROUTE_KEY_LEN: usize = 64;

/// Validate a route `key` (lowercase letters, digits, and hyphens; no
/// leading/trailing hyphen; max 64 chars). Mirrors the DB CHECK constraint
/// on `model_router_routes.key`.
pub fn validate_route_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("route key must not be empty".into());
    }
    if key.len() > MAX_ROUTE_KEY_LEN {
        return Err(format!(
            "route key must be at most {MAX_ROUTE_KEY_LEN} characters"
        ));
    }
    if !key
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err("route key must contain only lowercase letters, digits, and hyphens".into());
    }
    if key.starts_with('-') || key.ends_with('-') {
        return Err("route key must not start or end with a hyphen".into());
    }
    Ok(())
}

/// Validate a candidate's structural shape (without DB access). Domain-level
/// cross-validation (model belongs to caller's org, route exists, etc.) is
/// performed at the server layer.
pub fn validate_candidate_shape(
    candidate: &ModelRouterCandidate,
    strategy: ModelRouterStrategy,
) -> Result<(), String> {
    if candidate.weight < 0 {
        return Err(format!(
            "candidate.weight must be non-negative, got {}",
            candidate.weight
        ));
    }
    match strategy {
        ModelRouterStrategy::Rules => {
            if candidate.rules.is_none() {
                return Err(
                    "candidates under a 'rules' strategy must have a rules document set"
                        .to_string(),
                );
            }
        }
        ModelRouterStrategy::Single
        | ModelRouterStrategy::OrderedFallback
        | ModelRouterStrategy::Weighted
        | ModelRouterStrategy::Custom => {
            // No strategy-specific structural requirement on the candidate beyond weight.
        }
    }
    Ok(())
}

/// Validate a route's full shape (key + strategy + candidate count rules).
/// Cross-row uniqueness (route `key` per router, candidate ordering) is
/// enforced at the storage layer via unique indexes.
pub fn validate_route_shape(route: &ModelRouterRoute) -> Result<(), String> {
    validate_route_key(&route.key)?;
    if matches!(route.strategy, ModelRouterStrategy::Single) && route.candidates.len() != 1 {
        return Err(format!(
            "route '{}' has strategy 'single' but {} candidates; single-strategy routes must have exactly one candidate",
            route.key,
            route.candidates.len()
        ));
    }
    for candidate in &route.candidates {
        validate_candidate_shape(candidate, route.strategy)?;
    }
    Ok(())
}

/// OpenRouter-ready routing plan compiled from a model-router route.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenRouterRoutePlan {
    /// The concrete model slug to place in the required `model` request field.
    pub primary_model: String,
    /// Optional OpenRouter fallback routing fields. `None` means the route is a
    /// direct single-model invocation and no provider-specific fields are needed.
    pub routing: Option<OpenRouterRoutingConfig>,
}

/// Compile the currently executable Model Router strategies into OpenRouter's
/// request-level fallback routing fields.
///
/// `model_slug_for_candidate` resolves Everruns `ModelId` references to the
/// OpenRouter model slugs used on the wire (for example
/// `anthropic/claude-sonnet-4.5`). Storage-backed resolution lives outside this
/// foundational router module, so the caller supplies the lookup.
pub fn compile_openrouter_route_plan(
    route: &ModelRouterRoute,
    model_slug_for_candidate: impl Fn(&ModelRouterCandidate) -> Option<String>,
) -> Result<OpenRouterRoutePlan, String> {
    validate_route_shape(route)?;

    match route.strategy {
        ModelRouterStrategy::Single => {
            let candidate = route
                .candidates
                .first()
                .ok_or_else(|| format!("route '{}' has no candidates", route.key))?;
            let primary_model = model_slug_for_candidate(candidate).ok_or_else(|| {
                format!(
                    "route '{}' candidate '{}' does not resolve to an OpenRouter model slug",
                    route.key, candidate.id
                )
            })?;
            Ok(OpenRouterRoutePlan {
                primary_model,
                routing: None,
            })
        }
        ModelRouterStrategy::OrderedFallback => {
            let mut candidates = route.candidates.iter().collect::<Vec<_>>();
            candidates.sort_by_key(|candidate| candidate.position);

            let mut models = Vec::with_capacity(candidates.len());
            for candidate in candidates {
                let slug = model_slug_for_candidate(candidate).ok_or_else(|| {
                    format!(
                        "route '{}' candidate '{}' does not resolve to an OpenRouter model slug",
                        route.key, candidate.id
                    )
                })?;
                models.push(slug);
            }

            let primary_model = models
                .first()
                .cloned()
                .ok_or_else(|| format!("route '{}' has no candidates", route.key))?;
            Ok(OpenRouterRoutePlan {
                primary_model,
                routing: Some(OpenRouterRoutingConfig::fallback_models(models)),
            })
        }
        ModelRouterStrategy::Weighted
        | ModelRouterStrategy::Rules
        | ModelRouterStrategy::Custom => Err(format!(
            "route '{}' strategy '{}' cannot be compiled directly to OpenRouter fallback routing",
            route.key, route.strategy
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap()
    }

    fn candidate(weight: i32, rules: Option<serde_json::Value>) -> ModelRouterCandidate {
        candidate_with(weight, rules, 0, 1)
    }

    fn candidate_with(
        weight: i32,
        rules: Option<serde_json::Value>,
        position: i32,
        model_seed: u128,
    ) -> ModelRouterCandidate {
        ModelRouterCandidate {
            id: Uuid::from_u128(model_seed),
            model_id: ModelId::from_seed(model_seed),
            request_overrides: serde_json::Value::Null,
            weight,
            rules,
            position,
            created_at: now(),
            updated_at: now(),
        }
    }

    fn route(
        strategy: ModelRouterStrategy,
        candidates: Vec<ModelRouterCandidate>,
    ) -> ModelRouterRoute {
        ModelRouterRoute {
            id: Uuid::nil(),
            key: "base".into(),
            purpose: "default route".into(),
            when_to_use: "use this when no specific route fits".into(),
            strategy,
            position: 0,
            candidates,
            created_at: now(),
            updated_at: now(),
        }
    }

    #[test]
    fn status_round_trip() {
        assert_eq!(ModelRouterStatus::from("active").to_string(), "active");
        assert_eq!(ModelRouterStatus::from("archived").to_string(), "archived");
        assert_eq!(ModelRouterStatus::from("deleted").to_string(), "deleted");
        assert_eq!(ModelRouterStatus::from("unknown").to_string(), "active");
    }

    #[test]
    fn strategy_parse_round_trip() {
        for s in ["single", "ordered_fallback", "weighted", "rules", "custom"] {
            assert_eq!(ModelRouterStrategy::parse(s).unwrap().to_string(), s);
        }
    }

    #[test]
    fn strategy_parse_rejects_unknown() {
        let err = ModelRouterStrategy::parse("invalid").unwrap_err();
        assert!(err.contains("unknown model router strategy"));
    }

    #[test]
    fn route_key_accepts_canonical_keys() {
        for key in ["base", "utility", "analysis", "review", "fast-path", "v1"] {
            assert!(validate_route_key(key).is_ok(), "should accept key '{key}'");
        }
    }

    #[test]
    fn route_key_rejects_empty() {
        assert!(validate_route_key("").is_err());
    }

    #[test]
    fn route_key_rejects_uppercase() {
        assert!(validate_route_key("Analysis").is_err());
    }

    #[test]
    fn route_key_rejects_underscore() {
        assert!(validate_route_key("fast_path").is_err());
    }

    #[test]
    fn route_key_rejects_leading_hyphen() {
        assert!(validate_route_key("-fast").is_err());
    }

    #[test]
    fn route_key_rejects_trailing_hyphen() {
        assert!(validate_route_key("fast-").is_err());
    }

    #[test]
    fn route_key_rejects_too_long() {
        let key = "a".repeat(MAX_ROUTE_KEY_LEN + 1);
        assert!(validate_route_key(&key).is_err());
    }

    #[test]
    fn candidate_shape_rejects_negative_weight() {
        let cand = candidate(-1, None);
        assert!(validate_candidate_shape(&cand, ModelRouterStrategy::Weighted).is_err());
    }

    #[test]
    fn candidate_shape_rules_strategy_requires_rules_doc() {
        let cand = candidate(1, None);
        let err = validate_candidate_shape(&cand, ModelRouterStrategy::Rules).unwrap_err();
        assert!(err.contains("rules"));
    }

    #[test]
    fn candidate_shape_rules_strategy_accepts_rules_doc() {
        let cand = candidate(1, Some(serde_json::json!({ "if": { "tier": "fast" } })));
        assert!(validate_candidate_shape(&cand, ModelRouterStrategy::Rules).is_ok());
    }

    #[test]
    fn route_shape_rejects_single_with_multiple_candidates() {
        let route = ModelRouterRoute {
            id: Uuid::nil(),
            key: "base".into(),
            purpose: "default route".into(),
            when_to_use: "use this when no specific route fits".into(),
            strategy: ModelRouterStrategy::Single,
            position: 0,
            candidates: vec![candidate(1, None), candidate(1, None)],
            created_at: now(),
            updated_at: now(),
        };
        let err = validate_route_shape(&route).unwrap_err();
        assert!(err.contains("single"));
    }

    #[test]
    fn route_shape_rejects_single_with_zero_candidates() {
        let route = ModelRouterRoute {
            id: Uuid::nil(),
            key: "base".into(),
            purpose: "default route".into(),
            when_to_use: "use this when no specific route fits".into(),
            strategy: ModelRouterStrategy::Single,
            position: 0,
            candidates: vec![],
            created_at: now(),
            updated_at: now(),
        };
        let err = validate_route_shape(&route).unwrap_err();
        assert!(err.contains("single"));
    }

    #[test]
    fn route_shape_accepts_single_with_exactly_one_candidate() {
        let route = ModelRouterRoute {
            id: Uuid::nil(),
            key: "base".into(),
            purpose: "default route".into(),
            when_to_use: "use this when no specific route fits".into(),
            strategy: ModelRouterStrategy::Single,
            position: 0,
            candidates: vec![candidate(1, None)],
            created_at: now(),
            updated_at: now(),
        };
        assert!(validate_route_shape(&route).is_ok());
    }

    #[test]
    fn route_shape_accepts_ordered_fallback_with_multiple_candidates() {
        let route = route(
            ModelRouterStrategy::OrderedFallback,
            vec![candidate(1, None), candidate(1, None)],
        );
        assert!(validate_route_shape(&route).is_ok());
    }

    #[test]
    fn openrouter_plan_single_returns_primary_without_routing() {
        let route = route(ModelRouterStrategy::Single, vec![candidate(1, None)]);

        let plan = compile_openrouter_route_plan(&route, |candidate| {
            assert_eq!(candidate.model_id, ModelId::from_seed(1));
            Some("openai/gpt-5-mini".to_string())
        })
        .unwrap();

        assert_eq!(plan.primary_model, "openai/gpt-5-mini");
        assert_eq!(plan.routing, None);
    }

    #[test]
    fn openrouter_plan_ordered_fallback_preserves_candidate_order() {
        let route = route(
            ModelRouterStrategy::OrderedFallback,
            vec![
                candidate_with(1, None, 10, 2),
                candidate_with(1, None, 0, 1),
            ],
        );

        let plan = compile_openrouter_route_plan(&route, |candidate| {
            if candidate.model_id == ModelId::from_seed(1) {
                Some("openai/gpt-5-mini".to_string())
            } else if candidate.model_id == ModelId::from_seed(2) {
                Some("anthropic/claude-sonnet-4.5".to_string())
            } else {
                None
            }
        })
        .unwrap();

        assert_eq!(plan.primary_model, "openai/gpt-5-mini");
        let routing = plan.routing.unwrap();
        assert_eq!(
            routing.models,
            vec![
                "openai/gpt-5-mini".to_string(),
                "anthropic/claude-sonnet-4.5".to_string(),
            ]
        );
        assert_eq!(
            routing.route,
            Some(crate::driver_registry::OpenRouterRoute::Fallback)
        );
    }

    #[test]
    fn openrouter_plan_rejects_uncompiled_strategies() {
        let route = route(ModelRouterStrategy::Weighted, vec![candidate(1, None)]);

        let err = compile_openrouter_route_plan(&route, |_| Some("openai/gpt-5-mini".to_string()))
            .unwrap_err();

        assert!(err.contains("cannot be compiled directly"));
    }

    #[test]
    fn openrouter_plan_rejects_missing_model_slug() {
        let route = route(ModelRouterStrategy::Single, vec![candidate(1, None)]);

        let err = compile_openrouter_route_plan(&route, |_| None).unwrap_err();

        assert!(err.contains("does not resolve to an OpenRouter model slug"));
    }

    #[test]
    fn candidate_default_weight_is_one() {
        let json = r#"{
            "id": "00000000-0000-0000-0000-000000000000",
            "model_id": "model_00000000000000000000000000000001",
            "position": 0,
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;
        let cand: ModelRouterCandidate = serde_json::from_str(json).unwrap();
        assert_eq!(cand.weight, 1);
    }

    #[test]
    fn candidate_default_request_overrides_is_empty_object() {
        // Wire and DB shapes must match: missing `request_overrides` defaults
        // to `{}`, never JSON `null`, so downstream consumers can rely on an
        // object-shaped value.
        let json = r#"{
            "id": "00000000-0000-0000-0000-000000000000",
            "model_id": "model_00000000000000000000000000000001",
            "position": 0,
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;
        let cand: ModelRouterCandidate = serde_json::from_str(json).unwrap();
        assert!(
            cand.request_overrides.is_object(),
            "expected default request_overrides to be a JSON object, got {:?}",
            cand.request_overrides
        );
        assert_eq!(cand.request_overrides.as_object().unwrap().len(), 0);
    }

    #[test]
    fn router_default_param_schema_is_empty_object() {
        // Wire and DB shapes must match: missing `param_schema` defaults to
        // `{}`, never JSON `null`.
        let json = r#"{
            "id": "mrtr_00000000000000000000000000000001",
            "name": "default",
            "status": "active",
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;
        let router: ModelRouter = serde_json::from_str(json).unwrap();
        assert!(
            router.param_schema.is_object(),
            "expected default param_schema to be a JSON object, got {:?}",
            router.param_schema
        );
        assert_eq!(router.param_schema.as_object().unwrap().len(), 0);
    }
}
