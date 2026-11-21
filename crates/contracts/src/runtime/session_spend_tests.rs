use super::*;
use serde_json::json;

fn call(turn: Option<Uuid>, actual: Option<f64>, estimated: Option<f64>) -> SpendCall {
    SpendCall {
        turn_id: turn,
        provider: Some("openrouter".into()),
        model: Some("no-such-model".into()),
        input_tokens: 100,
        output_tokens: 10,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
        actual_cost_usd: actual,
        estimated_cost_usd: estimated,
    }
}

#[test]
fn turns_background_and_waste_are_split() {
    let ok = Uuid::now_v7();
    let failed = Uuid::now_v7();
    let inputs = SpendInputs {
        calls: vec![
            call(Some(ok), Some(0.10), None),
            call(Some(failed), Some(0.05), None),
            call(None, Some(0.01), None),
        ],
        turns: vec![(ok, "completed".into()), (failed, "failed".into())],
        ..Default::default()
    };
    let spend = summarize(
        "s".into(),
        &inputs,
        vec![],
        SubagentSpend::default(),
        Some(ok),
    );
    assert!((spend.models.usd - 0.15).abs() < 1e-12);
    assert_eq!(spend.models.calls, 2);
    assert_eq!(spend.models.reported_calls, 2);
    assert!((spend.background.usd - 0.01).abs() < 1e-12);
    assert_eq!(spend.background.calls, 1);
    assert!((spend.wasted_usd - 0.05).abs() < 1e-12);
    assert_eq!(spend.current_turn_usd, Some(0.10));
    assert!((spend.total_usd - 0.16).abs() < 1e-12);
    assert_eq!(spend.basis, SpendBasis::ProviderReported);
    assert!(!spend.partial);
    // A reported cost without a price table cannot be split by category.
    assert!((spend.models.unattributed_usd - 0.15).abs() < 1e-12);
}

#[test]
fn an_unpriced_call_marks_the_total_partial() {
    let turn = Uuid::now_v7();
    let inputs = SpendInputs {
        calls: vec![
            call(Some(turn), None, Some(0.02)),
            call(Some(turn), None, None),
        ],
        ..Default::default()
    };
    let spend = summarize("s".into(), &inputs, vec![], SubagentSpend::default(), None);
    assert_eq!(spend.models.unpriced_calls, 1);
    assert!(spend.partial);
    assert_eq!(spend.basis, SpendBasis::ListPrice);
    assert_eq!(spend.current_turn_usd, None);
}

#[test]
fn sandboxes_and_subagents_add_to_the_total_with_the_weakest_basis() {
    let sandbox = SandboxSpend {
        provider: "daytona".into(),
        size: None,
        resources: None,
        running_seconds: 3600,
        stopped_seconds: 0,
        usd: Some(0.07),
        basis: SpendBasis::ListPrice,
    };
    let unpriced = SandboxSpend {
        provider: "boxd".into(),
        usd: None,
        basis: SpendBasis::Unpriced,
        ..sandbox.clone()
    };
    let inputs = SpendInputs {
        calls: vec![call(Some(Uuid::now_v7()), Some(0.1), None)],
        ..Default::default()
    };
    let spend = summarize(
        "s".into(),
        &inputs,
        vec![sandbox, unpriced],
        SubagentSpend {
            sessions: 1,
            usd: 0.03,
        },
        None,
    );
    assert!((spend.total_usd - 0.2).abs() < 1e-12);
    assert_eq!(spend.basis, SpendBasis::ListPrice);
    assert!(spend.partial, "a running sandbox without a price");
}

#[test]
fn image_results_are_priced_from_their_usage() {
    let tool_completed = json!({
        "tool_name": "generate_image",
        "success": true,
        "result": [{"type": "text", "text": json!({
            "provider": "openai", "model": "gpt-image-2.6-flare", "count": 1,
            "usage": {"input_text_tokens": 1000, "input_image_tokens": 0, "output_tokens": 4000}
        }).to_string()}]
    });
    let result = image_tool_result(&tool_completed).expect("image result");
    let usd = image_result_cost(&result).expect("priced");
    assert!(usd > 0.0);
    let spend = summarize(
        "s".into(),
        &SpendInputs {
            image_results: vec![result],
            ..Default::default()
        },
        vec![],
        SubagentSpend::default(),
        None,
    );
    assert_eq!((spend.images.calls, spend.images.images), (1, 1));
    assert!((spend.total_usd - usd).abs() < 1e-12);

    let other = json!({"tool_name": "web_fetch", "success": true, "result": []});
    assert!(image_tool_result(&other).is_none());
}

#[test]
fn split_scales_the_price_table_to_the_recorded_cost() {
    let parts = CostBreakdown {
        uncached_input: 1.0,
        cache_read: 0.0,
        cache_write: 0.0,
        output: 3.0,
    };
    let (split, rest) = split_call(2.0, Some(parts));
    assert_eq!((split.uncached_input, split.output, rest), (0.5, 1.5, 0.0));
}
