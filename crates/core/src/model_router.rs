// Model Router domain types
//
// Design intent lives in `knowledge/integrations/model-router.md`.
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

/// Selection strategy for a route. See `knowledge/integrations/model-router.md` for behavior.
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
    fn status_wire_values_and_legacy_fallback_are_explicit() {
        for (status, wire) in [
            (ModelRouterStatus::Active, "active"),
            (ModelRouterStatus::Archived, "archived"),
            (ModelRouterStatus::Deleted, "deleted"),
        ] {
            assert_eq!(status.to_string(), wire);
            assert_eq!(ModelRouterStatus::from(wire), status);
            assert_eq!(
                serde_json::to_value(status).unwrap(),
                serde_json::json!(wire)
            );
            assert_eq!(
                serde_json::from_value::<ModelRouterStatus>(serde_json::json!(wire)).unwrap(),
                status
            );
        }
        assert_eq!(
            ModelRouterStatus::from("unknown"),
            ModelRouterStatus::Active
        );
        assert!(serde_json::from_value::<ModelRouterStatus>(serde_json::json!("unknown")).is_err());
    }

    #[test]
    fn strategy_parser_and_serialization_agree_with_literal_variants() {
        for (strategy, wire) in [
            (ModelRouterStrategy::Single, "single"),
            (ModelRouterStrategy::OrderedFallback, "ordered_fallback"),
            (ModelRouterStrategy::Weighted, "weighted"),
            (ModelRouterStrategy::Rules, "rules"),
            (ModelRouterStrategy::Custom, "custom"),
        ] {
            assert_eq!(ModelRouterStrategy::parse(wire).unwrap(), strategy);
            assert_eq!(strategy.to_string(), wire);
            assert_eq!(
                serde_json::to_value(strategy).unwrap(),
                serde_json::json!(wire)
            );
            assert_eq!(
                serde_json::from_value::<ModelRouterStrategy>(serde_json::json!(wire)).unwrap(),
                strategy
            );
        }
        for bad in ["invalid", "", "Single", " single "] {
            assert!(
                ModelRouterStrategy::parse(bad)
                    .unwrap_err()
                    .contains("unknown model router strategy")
            );
            assert!(serde_json::from_value::<ModelRouterStrategy>(serde_json::json!(bad)).is_err());
        }
    }

    #[test]
    fn route_keys_enforce_literal_length_and_character_boundaries() {
        for key in [
            "base",
            "utility",
            "analysis",
            "review",
            "fast-path",
            "v1",
            "a",
            "7",
            "a--b",
            &"a".repeat(64),
        ] {
            assert!(validate_route_key(key).is_ok(), "{key}");
        }
        for key in [
            "",
            "Analysis",
            "fast_path",
            "-fast",
            "fast-",
            "-",
            "a/b",
            "a b",
            "é",
            &"a".repeat(65),
        ] {
            assert!(validate_route_key(key).is_err(), "{key:?}");
        }
    }

    #[test]
    fn candidate_validation_enforces_weight_for_every_strategy_and_rules_presence() {
        for strategy in [
            ModelRouterStrategy::Single,
            ModelRouterStrategy::OrderedFallback,
            ModelRouterStrategy::Weighted,
            ModelRouterStrategy::Rules,
            ModelRouterStrategy::Custom,
        ] {
            let rules = (strategy == ModelRouterStrategy::Rules)
                .then(|| serde_json::json!({"if":{"tier":"fast"}}));
            for weight in [0, 1, i32::MAX] {
                assert!(
                    validate_candidate_shape(&candidate(weight, rules.clone()), strategy).is_ok(),
                    "{strategy:?}/{weight}"
                );
            }
            for weight in [-1, i32::MIN] {
                assert!(
                    validate_candidate_shape(&candidate(weight, rules.clone()), strategy)
                        .unwrap_err()
                        .contains("weight")
                );
            }
        }
        assert!(
            validate_candidate_shape(&candidate(1, None), ModelRouterStrategy::Rules)
                .unwrap_err()
                .contains("rules")
        );
    }

    #[test]
    fn route_validation_checks_cardinality_keys_and_every_candidate() {
        for count in [0, 1, 2] {
            let route = route(
                ModelRouterStrategy::Single,
                (0..count).map(|_| candidate(1, None)).collect(),
            );
            assert_eq!(
                validate_route_shape(&route).is_ok(),
                count == 1,
                "count={count}"
            );
        }
        let mut fallback = route(
            ModelRouterStrategy::OrderedFallback,
            vec![candidate(1, None), candidate(1, None)],
        );
        assert!(validate_route_shape(&fallback).is_ok());
        fallback.candidates[1].weight = -1;
        assert!(
            validate_route_shape(&fallback)
                .unwrap_err()
                .contains("weight")
        );
        fallback.candidates[1].weight = 1;
        fallback.key = "Bad-Key".into();
        assert!(validate_route_shape(&fallback).is_err());
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
            serde_json::to_value(&routing).unwrap(),
            serde_json::json!({"models":["openai/gpt-5-mini","anthropic/claude-sonnet-4.5"],"route":"fallback"})
        );
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
    fn openrouter_plan_rejects_uncompiled_strategies_without_resolving_models() {
        for strategy in [
            ModelRouterStrategy::Weighted,
            ModelRouterStrategy::Rules,
            ModelRouterStrategy::Custom,
        ] {
            let route = route(
                strategy,
                vec![candidate(
                    1,
                    Some(serde_json::json!({"if":{"tier":"fast"}})),
                )],
            );
            let error = compile_openrouter_route_plan(&route, |_| {
                panic!("unsupported strategy must not resolve models")
            })
            .unwrap_err();
            assert!(
                error.contains("cannot be compiled directly"),
                "{strategy:?}: {error}"
            );
        }
    }

    #[test]
    fn openrouter_plan_rejects_missing_primary_or_fallback_slug() {
        for strategy in [
            ModelRouterStrategy::Single,
            ModelRouterStrategy::OrderedFallback,
        ] {
            let route = route(strategy, vec![candidate(1, None)]);
            let error = compile_openrouter_route_plan(&route, |_| None).unwrap_err();
            assert!(error.contains("does not resolve to an OpenRouter model slug"));
            assert!(error.contains(&route.candidates[0].id.to_string()));
        }
        let route = route(
            ModelRouterStrategy::OrderedFallback,
            vec![candidate_with(1, None, 0, 1), candidate_with(1, None, 1, 2)],
        );
        let calls = std::cell::RefCell::new(Vec::new());
        let error = compile_openrouter_route_plan(&route, |candidate| {
            calls.borrow_mut().push(candidate.id);
            (candidate.model_id == ModelId::from_seed(1)).then(|| "first/model".into())
        })
        .unwrap_err();
        assert_eq!(
            *calls.borrow(),
            vec![Uuid::from_u128(1), Uuid::from_u128(2)]
        );
        assert!(error.contains(&Uuid::from_u128(2).to_string()));
    }

    #[test]
    fn invalid_or_empty_routes_fail_before_model_resolution() {
        let mut invalid_key = route(ModelRouterStrategy::Single, vec![candidate(1, None)]);
        invalid_key.key = "Bad".into();
        for route in [
            invalid_key,
            route(ModelRouterStrategy::Single, vec![]),
            route(ModelRouterStrategy::OrderedFallback, vec![]),
        ] {
            assert!(
                compile_openrouter_route_plan(&route, |_| panic!(
                    "invalid route must not resolve models"
                ))
                .is_err()
            );
        }
    }

    #[test]
    fn candidate_wire_defaults_and_explicit_overrides_survive_serialization() {
        let minimal = serde_json::json!({"id":"00000000-0000-0000-0000-000000000000","model_id":"model_00000000000000000000000000000001","position":0,"created_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"});
        let parsed: ModelRouterCandidate = serde_json::from_value(minimal.clone()).unwrap();
        let mut expected = minimal.clone();
        expected["weight"] = serde_json::json!(1);
        expected["request_overrides"] = serde_json::json!({});
        assert_eq!(serde_json::to_value(parsed).unwrap(), expected);
        expected["weight"] = serde_json::json!(0);
        expected["request_overrides"] = serde_json::json!({"temperature":0.25});
        expected["rules"] = serde_json::json!({"tier":"fast"});
        let explicit: ModelRouterCandidate = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(explicit).unwrap(), expected);
    }

    #[test]
    fn router_wire_defaults_omit_internal_identity_and_empty_routes() {
        let minimal = serde_json::json!({"id":"mrtr_00000000000000000000000000000001","name":"default","status":"active","created_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"});
        let mut router: ModelRouter = serde_json::from_value(minimal.clone()).unwrap();
        assert_eq!(router.internal_id, Uuid::nil());
        router.internal_id = Uuid::from_u128(99);
        let mut expected = minimal;
        expected["param_schema"] = serde_json::json!({});
        assert_eq!(serde_json::to_value(&router).unwrap(), expected);
        router.param_schema =
            serde_json::json!({"type":"object","properties":{"tier":{"type":"string"}}});
        router.description = Some("Choose by tier".into());
        expected["param_schema"] = router.param_schema.clone();
        expected["description"] = serde_json::json!("Choose by tier");
        assert_eq!(serde_json::to_value(&router).unwrap(), expected);
    }
}
