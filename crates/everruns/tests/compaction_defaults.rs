//! Default harnesses and the no-harness Framework lane share managed continuity.
#![cfg(feature = "builtins")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
use everruns::hosting::{AgentBuild, MemoryHosting};
use everruns::{Agent, Engine, Harness, Model, SessionId};

fn agent() -> Agent {
    Agent::builder()
        .instructions("Remember constraints and retrieve evidence when needed.")
        .model(Model::simulated("retained"))
        .build()
        .unwrap()
}

#[tokio::test]
async fn every_framework_preset_exposes_effective_compaction_and_history_retrieval() {
    for harness in [
        Harness::base(),
        Harness::conversation(),
        Harness::worker(),
        Harness::bashkit_worker(),
    ] {
        let build = AgentBuild::new(harness.name(), "v1");
        let engine = Engine::builder()
            .hosting(MemoryHosting::default())
            .register_with_harness(build.clone(), agent(), harness)
            .build();
        let session = engine
            .create_registered(SessionId::new(), &build)
            .await
            .unwrap();
        let context = session.inspect().await.unwrap();
        let policy = context.compaction;
        assert!(policy.enabled && policy.proactive && policy.history_retrieval_available);
        assert_eq!(policy.strategy.as_deref(), Some("auto"));
        assert_eq!(policy.context_budget_tokens, Some(64_000));
        assert_eq!(policy.checkpoint_store_available, Some(true));
        assert!(policy.checkpoint.is_none());
        assert!(
            context
                .tools
                .iter()
                .any(|tool| tool.name == "query_history")
        );
        assert!(
            session.history().page().await.unwrap().messages.is_empty(),
            "inspection never starts a turn"
        );
    }
}

#[tokio::test]
async fn no_harness_compacts_across_turns_and_inspection_reuses_the_checkpoint() {
    let build = AgentBuild::new("default-continuity", "v1");
    let engine = Engine::builder()
        .hosting(MemoryHosting::default())
        .register(build.clone(), agent())
        .build();
    let session = engine
        .create_registered(SessionId::new(), &build)
        .await
        .unwrap();
    assert!(session.inspect().await.unwrap().compaction.enabled);
    for index in 0..16 {
        // Enough pressure for the default 64K budget, while keeping a recent
        // complete-exchange suffix below it. The simulator has no external calls.
        session
            .send(format!("record {index}: {}", "evidence ".repeat(2500)))
            .await
            .unwrap()
            .wait()
            .await
            .unwrap();
    }
    let before = session.inspect().await.unwrap().compaction;
    let checkpoint = before
        .checkpoint
        .expect("default policy installs continuity");
    assert_eq!(checkpoint.kind, "portable_summary");
    assert!(
        session.events_after(0).await.unwrap().iter().any(|event| {
            everruns::observe_compaction(event).is_some_and(|observation| {
                observation.verdict == everruns::CompactionVerdict::Passed
                    && observation.continuity_eligible
            })
        }),
        "the default Framework lane exposes host-owned monitoring evidence"
    );
    let again = session
        .inspect()
        .await
        .unwrap()
        .compaction
        .checkpoint
        .unwrap();
    assert_eq!(
        again.id, checkpoint.id,
        "inspection cannot generate a new summary"
    );
    session
        .send("Continue with retained constraints.")
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_eq!(
        session
            .inspect()
            .await
            .unwrap()
            .compaction
            .checkpoint
            .unwrap()
            .id,
        checkpoint.id
    );
    assert!(
        session.history().page().await.unwrap().messages.len() >= 34,
        "original messages stay available"
    );
}

#[tokio::test]
async fn explicit_agent_configuration_and_custom_harness_keep_authority() {
    let configured = Agent::builder()
        .instructions("Inspect only.")
        .model(Model::simulated("ok"))
        .capability(everruns::CapabilityRef::with_config(
            "compaction",
            serde_json::json!({"context_budget_tokens":8192,"proactive":false}),
        ))
        .build()
        .unwrap();
    let engine = Engine::new();
    let policy = engine
        .create(configured)
        .inspect()
        .await
        .unwrap()
        .compaction;
    assert_eq!(policy.context_budget_tokens, Some(8192));
    assert!(!policy.proactive);
    let build = AgentBuild::new("custom", "v1");
    let engine = Engine::builder()
        .hosting(MemoryHosting::default())
        .register_with_harness(
            build.clone(),
            agent(),
            Harness::builder("custom").build().unwrap(),
        )
        .build();
    assert!(
        !engine
            .create_registered(SessionId::new(), &build)
            .await
            .unwrap()
            .inspect()
            .await
            .unwrap()
            .compaction
            .enabled
    );
}

#[tokio::test]
async fn an_irreducible_prefix_finishes_instead_of_replaying_forever() {
    let agent = Agent::builder()
        .instructions("immutable requirement ".repeat(10000))
        .model(Model::simulated_with_config(
            everruns::LlmSimConfig::fixed("ok").with_context_window(2000),
        ))
        .build()
        .unwrap();
    let engine = Engine::builder()
        .hosting(MemoryHosting::default())
        .register(AgentBuild::new("oversized", "v1"), agent)
        .build();
    let session = engine
        .create_registered(SessionId::new(), &AgentBuild::new("oversized", "v1"))
        .await
        .unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        session.send_and_wait("Continue."),
    )
    .await
    .expect("an unchanged oversized request must terminate");
    assert!(result.is_err() || !result.unwrap().success);
}
