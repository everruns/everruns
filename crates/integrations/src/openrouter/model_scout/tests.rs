use super::*;
use crate::openrouter::capabilities::Capability;

fn scout_blueprint() -> AgentBlueprint {
    ModelScoutCapability
        .agent_blueprints()
        .pop()
        .expect("model scout contributes a blueprint")
}

#[test]
fn config_schema_is_derived_from_scout_config() {
    let blueprint = scout_blueprint();
    let schema = blueprint
        .config_schema
        .expect("scout declares a config schema");
    let properties = schema["properties"]
        .as_object()
        .expect("derived schema exposes properties");

    // Every ScoutConfig field reaches the schema, with the runtime bounds
    // attached — no hand-maintained copy to drift.
    assert_eq!(properties.len(), 6);
    assert_eq!(properties["model"]["minLength"], 1);
    assert_eq!(
        properties["max_candidates"]["maximum"],
        MAX_PROBE_CANDIDATES
    );
    assert_eq!(
        properties["max_candidates"]["default"],
        DEFAULT_PROBE_CANDIDATES
    );
    assert_eq!(
        properties["probe_timeout_ms"]["minimum"],
        MIN_PROBE_TIMEOUT_MS
    );
    assert_eq!(
        properties["probe_timeout_ms"]["maximum"],
        MAX_PROBE_TIMEOUT_MS
    );
    assert_eq!(properties["max_spend_usd"]["maximum"], MAX_PROBE_SPEND_USD);
    assert_eq!(properties["probe_tasks"]["maxItems"], MAX_PROBE_TASKS);
    assert_eq!(schema["additionalProperties"], json!(false));
}

#[test]
fn scout_config_defaults_round_trip_through_the_schema() {
    let blueprint = scout_blueprint();
    let defaults = serde_json::to_value(ScoutConfig::default()).expect("serialize defaults");

    blueprint
        .validate_config(Some(&defaults))
        .expect("defaults must satisfy the blueprint's own schema");
}

#[test]
fn scout_config_bounds_are_enforced_at_spawn() {
    let blueprint = scout_blueprint();

    assert!(
        blueprint
            .validate_config(Some(&json!({"max_candidates": 5})))
            .is_ok()
    );
    assert!(
        blueprint
            .validate_config(Some(&json!({"model": "approved/model"})))
            .is_ok(),
        "the Default blueprint model must remain host-overridable"
    );
    assert!(
        blueprint
            .validate_config(Some(&json!({"model": ""})))
            .is_err(),
        "an empty model override must be rejected"
    );
    // Previously the schema described these bounds and nothing checked them.
    assert!(
        blueprint
            .validate_config(Some(&json!({"max_candidates": 5_000})))
            .is_err(),
        "max_candidates above the cap must be rejected"
    );
    assert!(
        blueprint
            .validate_config(Some(&json!({"probe_timeout_ms": 1})))
            .is_err(),
        "probe timeout below the floor must be rejected"
    );
    assert!(
        blueprint
            .validate_config(Some(&json!({"unknown_knob": true})))
            .is_err(),
        "unknown config keys must be rejected"
    );
}

fn make_probe(
    model_id: &str,
    task_id: &str,
    success: bool,
    latency_ms: u64,
    cost: Option<f64>,
) -> ProbeResult {
    ProbeResult {
        model_id: model_id.to_string(),
        task_id: task_id.to_string(),
        success,
        latency_ms,
        input_tokens: Some(10),
        output_tokens: Some(20),
        cost_usd: cost,
        error: if success {
            None
        } else {
            Some("error".to_string())
        },
        passed_checks: if success {
            vec!["not_empty".to_string()]
        } else {
            vec![]
        },
        failed_checks: if success {
            vec![]
        } else {
            vec!["not_empty".to_string()]
        },
    }
}

#[test]
fn rank_results_orders_by_score() {
    let results = vec![
        // slow_model: 1/3 success, 12s avg, cheap
        make_probe("slow_model", "t1", false, 12_000, Some(0.0001)),
        make_probe("slow_model", "t2", false, 11_000, Some(0.0001)),
        make_probe("slow_model", "t3", true, 13_000, Some(0.0001)),
        // fast_model: 3/3 success, 500ms avg, more expensive
        make_probe("fast_model", "t1", true, 500, Some(0.001)),
        make_probe("fast_model", "t2", true, 400, Some(0.001)),
        make_probe("fast_model", "t3", true, 600, Some(0.001)),
    ];

    let rankings = rank_results(&results);
    assert_eq!(rankings.len(), 2);
    // fast_model should win despite higher cost, because success rate dominates
    assert_eq!(rankings[0].model_id, "fast_model");
    assert_eq!(rankings[1].model_id, "slow_model");
    assert!(rankings[0].score > rankings[1].score);
}

#[test]
fn rank_results_empty_input_returns_empty() {
    assert!(rank_results(&[]).is_empty());
}

#[test]
fn rank_results_single_model() {
    let results = vec![
        make_probe("only_model", "t1", true, 1_000, Some(0.001)),
        make_probe("only_model", "t2", true, 2_000, Some(0.001)),
    ];
    let rankings = rank_results(&results);
    assert_eq!(rankings.len(), 1);
    assert_eq!(rankings[0].model_id, "only_model");
    assert_eq!(rankings[0].success_rate, 1.0);
    assert_eq!(rankings[0].probe_count, 2);
}

#[test]
fn compute_score_perfect_is_high() {
    let r = ModelRanking {
        model_id: "m".to_string(),
        display_name: None,
        success_rate: 1.0,
        avg_latency_ms: 100.0,
        total_cost_usd: 0.0001,
        probe_count: 3,
        score: 0.0,
    };
    let s = compute_score(&r, 0.01);
    // success=1.0*0.5 + latency=(1-100/15000)*0.3 + cost=1.0*0.2
    assert!(s > 0.9, "perfect model should score above 0.9, got {s}");
}

#[test]
fn compute_score_zero_success_is_low() {
    let r = ModelRanking {
        model_id: "m".to_string(),
        display_name: None,
        success_rate: 0.0,
        avg_latency_ms: 15_000.0,
        total_cost_usd: 1.0,
        probe_count: 5,
        score: 0.0,
    };
    let s = compute_score(&r, 0.2);
    // 0.0*0.5 + 0.0*0.3 + 0.0*0.2 = 0.0
    assert_eq!(
        s, 0.0,
        "zero success + max latency + max cost should score 0"
    );
}

#[test]
fn compute_score_cost_zero_max_cost_zero() {
    // When max_cost_per_probe is 0, cost_score should be 1.0 (no penalty)
    let r = ModelRanking {
        model_id: "m".to_string(),
        display_name: None,
        success_rate: 1.0,
        avg_latency_ms: 0.0,
        total_cost_usd: 0.0,
        probe_count: 1,
        score: 0.0,
    };
    let s = compute_score(&r, 0.0);
    assert!((s - 1.0).abs() < 1e-9, "score should be 1.0, got {s}");
}

#[test]
fn rank_results_prefers_lower_latency_among_equal_success() {
    let results = vec![
        make_probe("model_a", "t1", true, 5_000, Some(0.001)),
        make_probe("model_b", "t1", true, 1_000, Some(0.001)),
    ];
    let rankings = rank_results(&results);
    assert_eq!(
        rankings[0].model_id, "model_b",
        "lower latency should rank higher when success rates are equal"
    );
}

#[test]
fn default_probe_tasks_non_empty() {
    let tasks = default_probe_tasks();
    assert!(!tasks.is_empty());
    for t in &tasks {
        assert!(!t.id.is_empty());
        assert!(!t.prompt.is_empty());
    }
}

#[test]
fn model_scout_capability_is_high_risk() {
    assert_eq!(ModelScoutCapability.risk_level(), RiskLevel::High);
}

#[test]
fn probe_timeout_is_clamped_to_schema_bounds() {
    assert_eq!(
        bounded_probe_timeout_ms(&json!({})),
        DEFAULT_PROBE_TIMEOUT_MS
    );
    assert_eq!(
        bounded_probe_timeout_ms(&json!({ "timeout_ms": 1 })),
        MIN_PROBE_TIMEOUT_MS
    );
    assert_eq!(
        bounded_probe_timeout_ms(&json!({ "timeout_ms": 3_600_000 })),
        MAX_PROBE_TIMEOUT_MS
    );
}

#[test]
fn probe_spend_is_clamped_to_schema_bounds() {
    assert_eq!(
        bounded_probe_max_spend_usd(&json!({})),
        DEFAULT_MAX_SPEND_USD
    );
    assert_eq!(
        bounded_probe_max_spend_usd(&json!({ "max_spend_usd": -1.0 })),
        0.0
    );
    assert_eq!(
        bounded_probe_max_spend_usd(&json!({ "max_spend_usd": 1_000.0 })),
        MAX_PROBE_SPEND_USD
    );
}

#[test]
fn custom_probe_tasks_are_capped() {
    let tasks: Vec<ProbeTask> = (0..75)
        .map(|i| ProbeTask {
            id: format!("task_{i}"),
            prompt: "Reply OK".to_string(),
            checks: vec![],
        })
        .collect();

    let limited = limit_probe_tasks(tasks);
    assert_eq!(limited.len(), MAX_PROBE_TASKS);
    assert_eq!(limited[0].id, "task_0");
    assert_eq!(limited[MAX_PROBE_TASKS - 1].id, "task_49");
}

#[tokio::test]
async fn rank_models_tool_returns_sorted_rankings() {
    let tool = RankModelsTool;
    let results = json!([
        {
            "model_id": "cheap",
            "task_id": "t1",
            "success": true,
            "latency_ms": 800,
            "input_tokens": 10,
            "output_tokens": 10,
            "cost_usd": 0.0001,
            "error": null,
            "passed_checks": ["not_empty"],
            "failed_checks": []
        },
        {
            "model_id": "expensive",
            "task_id": "t1",
            "success": true,
            "latency_ms": 800,
            "input_tokens": 10,
            "output_tokens": 10,
            "cost_usd": 0.1,
            "error": null,
            "passed_checks": ["not_empty"],
            "failed_checks": []
        }
    ]);

    let out = tool.execute(json!({ "results": results })).await;
    assert!(out.is_success());
    let tool_result = out.into_tool_result("id", "rank_models");
    let content = tool_result.result.unwrap();
    let rankings = content["rankings"].as_array().unwrap();
    assert_eq!(rankings.len(), 2);
    // cheap should score higher due to lower cost
    assert_eq!(rankings[0]["model_id"].as_str().unwrap(), "cheap");
}

#[tokio::test]
async fn propose_router_update_tool_produces_proposal() {
    let tool = ProposeRouterUpdateTool;
    let rankings = json!([
        {
            "model_id": "best_model",
            "display_name": null,
            "success_rate": 1.0,
            "avg_latency_ms": 500.0,
            "total_cost_usd": 0.001,
            "probe_count": 3,
            "score": 0.95
        },
        {
            "model_id": "fallback_model",
            "display_name": null,
            "success_rate": 0.66,
            "avg_latency_ms": 2000.0,
            "total_cost_usd": 0.0005,
            "probe_count": 3,
            "score": 0.60
        }
    ]);

    let out = tool
        .execute(json!({
            "rankings": rankings,
            "route_key": "base",
            "top_n": 2
        }))
        .await;

    assert!(out.is_success());
    let tool_result = out.into_tool_result("id", "propose_router_update");
    let content = tool_result.result.unwrap();
    assert_eq!(content["route_key"].as_str().unwrap(), "base");
    let candidates = content["proposed_candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].as_str().unwrap(), "best_model");
    assert_eq!(candidates[1].as_str().unwrap(), "fallback_model");
}

#[tokio::test]
async fn propose_router_update_empty_rankings_returns_error() {
    let tool = ProposeRouterUpdateTool;
    let out = tool.execute(json!({ "rankings": [] })).await;
    assert!(out.is_error(), "empty rankings should return error");
}
