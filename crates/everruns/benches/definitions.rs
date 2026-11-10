//! Agent definitions as data: what does hydrating a definition cost?
//!
//! Data plane v2 plan step 3 (`knowledge/execution/data-plane-v2/`) moves
//! platform agents from code registered at startup to definitions the
//! session authority keeps and the Engine hydrates on activation, cached by
//! revision. Its gate is the hydration cost per activation:
//! - cold: an activation whose revision is not cached reads the definition
//!   from the authority, parses the package and builds the Agent;
//! - warm: an activation whose revision is cached costs a lookup, so a turn
//!   of a defined agent runs as fast as a turn of a registered one.
//!
//! The definition is shaped like a real platform agent: long instructions,
//! a few capabilities and starter files. One llmsim turn per session.
//!
//! Decisions:
//! - Cold is measured twice: the hydrator alone (pure CPU, what the design
//!   promises is "milliseconds"), and whole turns of sessions whose
//!   revision is never cached (a one-revision cache over two agents), which
//!   adds the authority read and the cache churn.
//! - Warm compares turns of a defined agent with turns of the same agent
//!   registered as code, in alternating rounds so drift hits both.
//! - `cargo test` runs a smoke with the same budgets; `cargo bench` more
//!   sessions.
//!
//! Usage:
//!   cargo bench -p everruns --bench definitions
//!   cargo bench -p everruns --bench definitions -- --sessions 5000

use std::time::{Duration, Instant};

use everruns::hosting::{AgentBuild, AgentDefinition, Hydrated, MemoryHosting};
use everruns::{Engine, LlmSimConfig, Model, SessionId};
use serde_json::json;

/// Hydrating one definition, parse and build, mean; the smoke runs
/// unoptimized, on shared CI machines.
const COLD_HYDRATION_BUDGET: Duration = Duration::from_millis(1);
const COLD_HYDRATION_BUDGET_SMOKE: Duration = Duration::from_millis(5);
/// Extra time per turn when the revision is not cached, as a share of a
/// registered agent's turn.
const COLD_TURN_OVERHEAD_SHARE: f64 = 0.25;
/// Extra time per turn of a cached revision, as a share of a registered
/// agent's turn. The expected value is a lookup; the margin is noise.
const WARM_TURN_OVERHEAD_SHARE: f64 = 0.03;
/// The smoke runs 100 unoptimized turns per round, where run-to-run noise
/// alone reaches 3%; it only catches a warm path that stopped being a lookup.
const WARM_TURN_OVERHEAD_SHARE_SMOKE: f64 = 0.10;

fn args() -> (bool, usize) {
    let mut smoke = true;
    let mut sessions = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bench" => smoke = false,
            "--sessions" => sessions = args.next().and_then(|v| v.parse().ok()),
            _ => {}
        }
    }
    (smoke, sessions.unwrap_or(if smoke { 300 } else { 3_000 }))
}

fn definition(agent: &str, revision: &str) -> AgentDefinition {
    let instructions = "You are a support agent for a hosting company. \
        Answer briefly, cite the runbook section you used, and ask before \
        any change to a customer's account. "
        .repeat(40);
    let files: Vec<_> = (0..5)
        .map(|n| json!({ "path": format!("runbooks/section-{n}.md"), "content": "Step. ".repeat(300) }))
        .collect();
    AgentDefinition {
        build: AgentBuild::new(agent, revision),
        runtime_agent: everruns::hosting::RuntimeAgent::new(json!({
            "name": agent,
            "description": "Customer support",
            "instructions": instructions,
            "tags": ["support", "tier-1"],
            "capabilities": ["current_time", "stateless_todo_list", { "ref": "compaction", "config": {} }],
            "files": files,
        })),
    }
}

/// Offline hydration: the package mapping with a simulated model.
fn hydrate(definition: &AgentDefinition) -> Result<Hydrated, everruns::AgentLoopError> {
    let package = definition.agent_package()?;
    let config = |error: String| everruns::AgentLoopError::config(error);
    let agent = package
        .builder()
        .map_err(|e| config(e.to_string()))?
        .model(Model::simulated_with_config(LlmSimConfig::lorem(20)))
        .build()
        .map_err(|e| config(e.to_string()))?;
    Ok(Hydrated {
        agent,
        harness: None,
    })
}

async fn turns(
    engine: &Engine,
    sessions: usize,
    mut create: impl FnMut(SessionId) -> AgentBuild,
) -> Duration {
    let started = Instant::now();
    for _ in 0..sessions {
        let id = SessionId::new();
        let session = engine
            .create_registered(id, &create(id))
            .await
            .expect("create session");
        let turn = session.send_and_wait("hi").await.expect("turn");
        assert!(turn.success, "{turn:?}");
    }
    started.elapsed() / sessions as u32
}

fn main() {
    let (smoke, sessions) = args();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    // Cold hydration alone.
    let support = definition("support", "r1");
    hydrate(&support).expect("valid definition");
    let rounds = if smoke { 200 } else { 2_000 };
    let started = Instant::now();
    for _ in 0..rounds {
        hydrate(&support).expect("hydrate");
    }
    let hydration = started.elapsed() / rounds;

    let (registered, warm, cold) = runtime.block_on(async {
        let registered_build = AgentBuild::new("support-code", "r1");
        let registered_engine = Engine::builder()
            .hosting(MemoryHosting::default().without_prompts())
            .build();
        let agent = hydrate(&support).unwrap().agent;
        let factory = agent.environment_factory();
        registered_engine
            .register_agent_with_environment(
                registered_build.clone(),
                agent,
                None,
                "agent-default",
                factory,
            )
            .unwrap();
        let warm_engine = Engine::builder()
            .hosting(MemoryHosting::default().without_prompts())
            .hydrator(hydrate)
            .build();
        warm_engine.publish_definition(&support).await.unwrap();
        // Two agents over a one-revision cache: every activation is cold.
        let cold_engine = Engine::builder()
            .hosting(MemoryHosting::default().without_prompts())
            .hydrator(hydrate)
            .hydrated_capacity(1)
            .build();
        let pair = [definition("support-a", "r1"), definition("support-b", "r1")];
        for definition in &pair {
            cold_engine.publish_definition(definition).await.unwrap();
        }
        // Warm up lazy statics on every path.
        turns(&registered_engine, 5, |_| registered_build.clone()).await;
        turns(&warm_engine, 5, |_| support.build.clone()).await;
        let (mut registered, mut warm, mut cold) = (Duration::ZERO, Duration::ZERO, Duration::ZERO);
        let round = sessions / 3;
        for _ in 0..3 {
            registered += turns(&registered_engine, round, |_| registered_build.clone()).await;
            warm += turns(&warm_engine, round, |_| support.build.clone()).await;
            let mut flip = false;
            cold += turns(&cold_engine, round, |_| {
                flip = !flip;
                pair[flip as usize].build.clone()
            })
            .await;
        }
        (registered / 3, warm / 3, cold / 3)
    });

    let micros = |d: Duration| d.as_secs_f64() * 1e6;
    println!(
        "definitions ({}): {sessions} sessions per path, one llmsim turn each",
        if smoke { "smoke" } else { "full" }
    );
    println!(
        "hydrate one definition (parse + build):  {:>9.1} us",
        micros(hydration)
    );
    println!(
        "turn, registered agent:                  {:>9.1} us",
        micros(registered)
    );
    println!(
        "turn, defined agent, revision cached:    {:>9.1} us",
        micros(warm)
    );
    println!(
        "turn, defined agent, revision not cached:{:>9.1} us",
        micros(cold)
    );
    let warm_overhead = warm.saturating_sub(registered);
    let cold_overhead = cold.saturating_sub(registered);
    println!(
        "warm overhead per turn:                  {:>9.1} us",
        micros(warm_overhead)
    );
    println!(
        "cold overhead per turn:                  {:>9.1} us",
        micros(cold_overhead)
    );
    let hydration_budget = if smoke {
        COLD_HYDRATION_BUDGET_SMOKE
    } else {
        COLD_HYDRATION_BUDGET
    };
    let warm_share = if smoke {
        WARM_TURN_OVERHEAD_SHARE_SMOKE
    } else {
        WARM_TURN_OVERHEAD_SHARE
    };
    assert!(
        hydration <= hydration_budget,
        "hydration {hydration:?}, budget {hydration_budget:?}"
    );
    assert!(
        warm_overhead.as_secs_f64() <= registered.as_secs_f64() * warm_share,
        "warm overhead {warm_overhead:?} over a {registered:?} turn"
    );
    assert!(
        cold_overhead.as_secs_f64() <= registered.as_secs_f64() * COLD_TURN_OVERHEAD_SHARE,
        "cold overhead {cold_overhead:?} over a {registered:?} turn"
    );
}
