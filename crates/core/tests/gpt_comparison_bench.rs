// GPT-5.2 vs GPT-5.4 comparison benchmark.
//
// Runs identical prompts against both models and compares latency, token usage,
// and iteration counts. Results are printed as a table for manual review.
//
// Run:
//   cargo test -p everruns-core --test gpt_comparison_bench -- --nocapture
//
// Required env: OPENAI_API_KEY
// Skip: SKIP_LLM_INTEGRATION_TESTS_PROVIDERS=openai
#![cfg(feature = "llm-tests")]

mod llm_test_matrix;

use llm_test_matrix::*;
use std::time::Instant;

use everruns_core::capabilities::CurrentTimeCapability;
use everruns_core::in_memory_loop::InMemoryAgenticLoop;
use everruns_core::message::{ContentPart, Controls, MessageRole, ReasoningConfig};
use everruns_core::message_retriever::InputMessage;

// ============================================================================
// Helpers
// ============================================================================

#[derive(Debug, Clone)]
struct BenchResult {
    model: String,
    scenario: String,
    latency_ms: u128,
    iterations: usize,
    tool_calls: usize,
    response_len: usize,
    success: bool,
}

fn print_comparison(results: &[BenchResult]) {
    eprintln!();
    eprintln!(
        "{:<12} {:<24} {:>10} {:>6} {:>6} {:>8} {:>7}",
        "MODEL", "SCENARIO", "LATENCY", "ITERS", "TOOLS", "RESP_LEN", "OK"
    );
    eprintln!("{}", "-".repeat(80));
    for r in results {
        eprintln!(
            "{:<12} {:<24} {:>8}ms {:>6} {:>6} {:>8} {:>7}",
            r.model,
            r.scenario,
            r.latency_ms,
            r.iterations,
            r.tool_calls,
            r.response_len,
            r.success,
        );
    }
    eprintln!();

    // Print speedup summary per scenario
    let scenarios: Vec<&str> = results
        .iter()
        .map(|r| r.scenario.as_str())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    for scenario in scenarios {
        let gpt52 = results
            .iter()
            .find(|r| r.model.contains("5.2") && r.scenario == scenario);
        let gpt54 = results
            .iter()
            .find(|r| r.model.contains("5.4") && r.scenario == scenario);
        if let (Some(a), Some(b)) = (gpt52, gpt54) {
            let speedup = a.latency_ms as f64 / b.latency_ms.max(1) as f64;
            eprintln!(
                "  {}: gpt-5.4 is {:.2}x {} than gpt-5.2 ({} ms vs {} ms)",
                scenario,
                if speedup >= 1.0 {
                    speedup
                } else {
                    1.0 / speedup
                },
                if speedup >= 1.0 { "faster" } else { "slower" },
                b.latency_ms,
                a.latency_ms,
            );
        }
    }
    eprintln!();
}

// ============================================================================
// Scenario: basic completion comparison
// ============================================================================

#[tokio::test]
async fn test_gpt52_vs_gpt54_basic_completion() {
    let configs = [("gpt-5.2", OPENAI_GPT52), ("gpt-5.4", OPENAI_GPT54)];
    let mut results = Vec::new();

    for (label, config) in &configs {
        let Some(model) = config.model() else {
            eprintln!("Skipping {}: {} not set", label, config.label());
            return;
        };

        let runner = InMemoryAgenticLoop::builder()
            .agent_name("Bench Agent")
            .system_prompt("Answer concisely in 1-2 sentences.")
            .model(model)
            .driver_registry(all_providers_registry())
            .max_iterations(3)
            .build()
            .await
            .unwrap();

        let start = Instant::now();
        let result = runner
            .run_turn("What is the capital of Japan and why was it chosen?")
            .await
            .unwrap();
        let elapsed = start.elapsed().as_millis();

        results.push(BenchResult {
            model: label.to_string(),
            scenario: "basic_completion".into(),
            latency_ms: elapsed,
            iterations: result.iterations,
            tool_calls: result.tool_calls_count,
            response_len: result.response.len(),
            success: result.success,
        });

        assert!(
            result.success,
            "{} basic completion failed: {:?}",
            label, result.error
        );
    }

    print_comparison(&results);
}

// ============================================================================
// Scenario: tool calling comparison
// ============================================================================

#[tokio::test]
async fn test_gpt52_vs_gpt54_tool_calling() {
    let configs = [("gpt-5.2", OPENAI_GPT52), ("gpt-5.4", OPENAI_GPT54)];
    let mut results = Vec::new();

    for (label, config) in &configs {
        let Some(model) = config.model() else {
            eprintln!("Skipping {}: {} not set", label, config.label());
            return;
        };

        let runner = InMemoryAgenticLoop::builder()
            .agent_name("Tool Bench Agent")
            .system_prompt("When asked about time, use get_current_time tool first.")
            .model(model)
            .driver_registry(all_providers_registry())
            .capability(CurrentTimeCapability)
            .max_iterations(5)
            .build()
            .await
            .unwrap();

        let start = Instant::now();
        let result = runner.run_turn("What time is it right now?").await.unwrap();
        let elapsed = start.elapsed().as_millis();

        results.push(BenchResult {
            model: label.to_string(),
            scenario: "tool_calling".into(),
            latency_ms: elapsed,
            iterations: result.iterations,
            tool_calls: result.tool_calls_count,
            response_len: result.response.len(),
            success: result.success,
        });

        assert!(
            result.success,
            "{} tool calling failed: {:?}",
            label, result.error
        );
        assert!(result.tool_calls_count > 0, "{} should call tool", label);
    }

    print_comparison(&results);
}

// ============================================================================
// Scenario: reasoning comparison (high effort)
// ============================================================================

#[tokio::test]
async fn test_gpt52_vs_gpt54_reasoning() {
    let configs = [("gpt-5.2", OPENAI_GPT52), ("gpt-5.4", OPENAI_GPT54)];
    let mut results = Vec::new();

    for (label, config) in &configs {
        let Some(model) = config.model() else {
            eprintln!("Skipping {}: {} not set", label, config.label());
            return;
        };

        let runner = InMemoryAgenticLoop::builder()
            .agent_name("Reasoning Bench Agent")
            .system_prompt("You are a helpful assistant.")
            .model(model)
            .driver_registry(all_providers_registry())
            .max_iterations(3)
            .build()
            .await
            .unwrap();

        let input = InputMessage {
            role: MessageRole::User,
            content: vec![ContentPart::text(
                "A farmer has 17 chickens. All but 9 die. How many are left? Think step by step.",
            )],
            controls: Some(Controls {
                model_id: None,
                reasoning: Some(ReasoningConfig {
                    effort: Some("high".into()),
                }),
                hints: None,
            }),
            metadata: None,
            tags: vec![],
        };

        let start = Instant::now();
        let result = runner.run_turn(input).await.unwrap();
        let elapsed = start.elapsed().as_millis();

        results.push(BenchResult {
            model: label.to_string(),
            scenario: "reasoning_high".into(),
            latency_ms: elapsed,
            iterations: result.iterations,
            tool_calls: result.tool_calls_count,
            response_len: result.response.len(),
            success: result.success,
        });

        assert!(
            result.success,
            "{} reasoning failed: {:?}",
            label, result.error
        );
        assert!(
            result.response.contains("9"),
            "{} should answer 9, got: {}",
            label,
            result.response
        );
    }

    print_comparison(&results);
}
