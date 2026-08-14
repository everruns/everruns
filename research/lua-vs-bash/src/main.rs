//! A/B evaluation harness: `lua` vs `virtual_bash` execution capability.
//!
//! Identical agent/model/prompt; the only difference between the two arms is
//! which execution capability the harness grants. Each task seeds a workspace
//! file, asks the agent to produce `/workspace/out.txt`, and a deterministic
//! grader checks the result. We record success, tool-call count, tool errors,
//! loop iterations, and latency per run.
//!
//! Run: `ANTHROPIC_API_KEY=… cargo run --manifest-path research/lua-vs-bash/Cargo.toml`
//! Optional: `EVAL_MODEL=claude-… EVAL_RUNS=2`.

use std::time::Instant;

use everruns_core::driver_registry::DriverRegistry;
use everruns_core::session_file::InitialFile;
use everruns_core::{
    AgentId, CapabilityRegistry, DriverId, HarnessId, MessageRole,
    provider_resolution::ResolvedModel, SessionId,
};
use everruns_host::HostComposition;
use everruns_host::{AgentBuilder, HarnessBuilder, InProcessRuntimeBuilder, SessionBuilder};
use everruns_integrations_bashkit::BashkitShellCapability;
use everruns_integrations_lua::LuaCapability;

const HARNESS_PROMPT: &str = "You are a data-processing assistant. You have exactly one \
code-execution tool. To complete a task you MUST use that tool to read and write files in \
/workspace — do not answer from your head. Always write the final answer to the file the task \
names, then stop.";

const AGENT_PROMPT: &str = "Use your execution tool to read inputs and write outputs under /workspace. \
Keep going until the requested output file exists, then stop.";

struct Task {
    id: &'static str,
    slice: &'static str,
    prompt: &'static str,
    seed_path: Option<&'static str>,
    seed_content: &'static str,
    check_path: &'static str,
    expect: &'static str,
}

fn tasks() -> Vec<Task> {
    vec![
        Task {
            id: "math",
            slice: "logic/math",
            prompt: "Compute 17 * 23 + 5 and write only the number to /workspace/out.txt.",
            seed_path: None,
            seed_content: "",
            check_path: "/workspace/out.txt",
            expect: "396",
        },
        Task {
            id: "json_sum",
            slice: "data/json",
            prompt: "Read the JSON array in /workspace/input.json. Sum the `amount` field of \
every element and write only the total to /workspace/out.txt.",
            seed_path: Some("/workspace/input.json"),
            seed_content: r#"[{"amount": 10}, {"amount": 32}, {"amount": 100}]"#,
            check_path: "/workspace/out.txt",
            expect: "142",
        },
        Task {
            id: "grep_count",
            slice: "vfs/grep",
            prompt: "Count how many lines in /workspace/log.txt contain the word ERROR and \
write only that count to /workspace/out.txt.",
            seed_path: Some("/workspace/log.txt"),
            seed_content: "INFO ok\nERROR boom\nINFO fine\nERROR again\nWARN meh\nERROR third\n",
            check_path: "/workspace/out.txt",
            expect: "3",
        },
        Task {
            id: "transform",
            slice: "data/transform",
            prompt: "Read /workspace/names.txt (one name per line). Write the same names to \
/workspace/out.txt sorted alphabetically, one per line.",
            seed_path: Some("/workspace/names.txt"),
            seed_content: "charlie\nalice\nbob\n",
            check_path: "/workspace/out.txt",
            expect: "alice",
        },
    ]
}

#[derive(Default, Clone)]
struct RunMetrics {
    graded_ok: bool,
    run_success: bool,
    tool_calls: usize,
    tool_errors: usize,
    iterations: usize,
    elapsed_ms: u128,
    error: Option<String>,
}

fn platform() -> HostComposition {
    let mut caps = CapabilityRegistry::new();
    caps.register(BashkitShellCapability);
    caps.register(LuaCapability);

    let mut drivers = DriverRegistry::new();
    everruns_anthropic::register_driver(&mut drivers);
    everruns_openai::register_driver(&mut drivers);
    everruns_test_support::llmsim_driver::register_driver(&mut drivers);

    HostComposition::new(caps, drivers)
}

async fn run_one(task: &Task, capability: &str, model: &ResolvedModel, seed: u64) -> RunMetrics {
    let harness_id = HarnessId::from_seed(seed as u128);
    let agent_id = AgentId::from_seed(seed as u128);
    let session_id = SessionId::from_seed(seed as u128);

    let mut harness = HarnessBuilder::new("eval", HARNESS_PROMPT)
        .id(harness_id)
        .capability(capability)
        .build();
    if let Some(path) = task.seed_path {
        harness.definition.initial_files.push(InitialFile {
            path: path.to_string(),
            content: task.seed_content.to_string(),
            encoding: "text".to_string(),
            is_readonly: false,
        });
    }

    let agent = AgentBuilder::new("eval-agent", AGENT_PROMPT)
        .id(agent_id)
        .max_iterations(10)
        .build();
    let session = SessionBuilder::new(harness_id)
        .id(session_id)
        .agent(agent_id)
        .build();

    let runtime = match InProcessRuntimeBuilder::new()
        .host_composition(platform())
        .default_model(model.clone())
        .harness(harness)
        .agent(agent)
        .session(session)
        .build()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return RunMetrics {
                error: Some(format!("build: {e}")),
                ..Default::default()
            };
        }
    };

    let start = Instant::now();
    let turn = runtime.run_text_turn(session_id, task.prompt).await;
    let elapsed_ms = start.elapsed().as_millis();

    let mut m = RunMetrics {
        elapsed_ms,
        ..Default::default()
    };
    match turn {
        Ok(t) => {
            m.run_success = t.success;
            m.iterations = t.iterations;
        }
        Err(e) => {
            m.error = Some(format!("turn: {e}"));
            return m;
        }
    }

    // Tool-call / error accounting from the persisted message log.
    if let Ok(messages) = runtime.messages(session_id).await {
        for msg in &messages {
            m.tool_calls += msg.tool_calls().len();
            if msg.role == MessageRole::ToolResult {
                let serialized = serde_json::to_string(msg).unwrap_or_default();
                if serialized.contains("\"error\"") {
                    m.tool_errors += 1;
                }
            }
        }
    }

    // Deterministic grade against the workspace.
    if let Ok(Some(file)) = runtime.read_file(session_id, task.check_path).await {
        let content = file.content.unwrap_or_default();
        m.graded_ok = content.contains(task.expect);
    }

    m
}

/// Print the fully assembled system prompt each arm sends to the model (harness
/// + agent + the execution capability's own contribution). No model call.
async fn dump_prompts(model: &ResolvedModel) -> anyhow::Result<()> {
    for capability in ["virtual_bash", "lua"] {
        let harness_id = HarnessId::from_seed(1);
        let agent_id = AgentId::from_seed(1);
        let session_id = SessionId::from_seed(1);
        let harness = HarnessBuilder::new("eval", HARNESS_PROMPT)
            .id(harness_id)
            .capability(capability)
            .build();
        let agent = AgentBuilder::new("eval-agent", AGENT_PROMPT)
            .id(agent_id)
            .max_iterations(10)
            .build();
        let session = SessionBuilder::new(harness_id)
            .id(session_id)
            .agent(agent_id)
            .build();
        let runtime = InProcessRuntimeBuilder::new()
            .host_composition(platform())
            .default_model(model.clone())
            .harness(harness)
            .agent(agent)
            .session(session)
            .build()
            .await?;
        let ctx = runtime.load_context(session_id).await?;
        let tools: Vec<_> = ctx.runtime_agent.tools.iter().map(|t| t.name()).collect();
        println!("\n========================================================");
        println!("ARM: {capability}  | tools: {tools:?}");
        println!("========== ASSEMBLED SYSTEM PROMPT ==========");
        println!("{}", ctx.runtime_agent.system_prompt);
        println!("========== END ==========");
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_else(|_| "dump-only".to_string());
    let model_name =
        std::env::var("EVAL_MODEL").unwrap_or_else(|_| "claude-haiku-4-5-20251001".to_string());
    let runs: u64 = std::env::var("EVAL_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);

    let model = ResolvedModel {
        model: model_name.clone(),
        provider_type: DriverId::Anthropic,
        api_key: Some(api_key),
        base_url: None,
        provider_metadata: None,
    };

    // `--dump-prompts` prints the assembled prompts and exits (no model call).
    if std::env::args().any(|a| a == "--dump-prompts") {
        return dump_prompts(&model).await;
    }
    if std::env::var("ANTHROPIC_API_KEY").is_err() {
        anyhow::bail!("set ANTHROPIC_API_KEY (Doppler) to run real-model evals");
    }

    let arms = ["virtual_bash", "lua"];
    let tasks = tasks();

    println!("# lua vs virtual_bash — model={model_name}, runs/task={runs}\n");
    println!("| task | slice | arm | graded | tool_calls | tool_errors | iters | ms | note |");
    println!("|---|---|---|---|---|---|---|---|---|");

    // arm -> (graded_ok, runs, tool_calls, tool_errors, iters, ms)
    let mut agg: std::collections::HashMap<&str, (usize, usize, usize, usize, usize, u128)> =
        std::collections::HashMap::new();
    let mut seed: u64 = 1000;

    for task in &tasks {
        for arm in arms {
            for _ in 0..runs {
                seed += 1;
                let m = run_one(task, arm, &model, seed).await;
                let e = agg.entry(arm).or_default();
                e.0 += m.graded_ok as usize;
                e.1 += 1;
                e.2 += m.tool_calls;
                e.3 += m.tool_errors;
                e.4 += m.iterations;
                e.5 += m.elapsed_ms;
                println!(
                    "| {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                    task.id,
                    task.slice,
                    arm,
                    if m.graded_ok { "✅" } else { "❌" },
                    m.tool_calls,
                    m.tool_errors,
                    m.iterations,
                    m.elapsed_ms,
                    m.error.as_deref().unwrap_or(""),
                );
            }
        }
    }

    println!("\n## Aggregate\n");
    println!("| arm | success | tool_calls | tool_errors | avg_iters | avg_ms |");
    println!("|---|---|---|---|---|---|");
    for arm in arms {
        if let Some(e) = agg.get(arm) {
            let n = e.1.max(1);
            println!(
                "| {} | {}/{} | {} | {} | {:.1} | {} |",
                arm,
                e.0,
                e.1,
                e.2,
                e.3,
                e.4 as f64 / n as f64,
                e.5 / n as u128,
            );
        }
    }

    Ok(())
}
