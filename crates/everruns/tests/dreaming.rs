//! Framework acceptance: capability execution and real mounted files, no server.
use everruns::{Agent, DreamDestination, DreamTriggerMode, DreamingConfig, Engine, Model};
fn config() -> DreamingConfig {
    let mut c = DreamingConfig::new(DreamDestination {
        mount: "/memory/user".into(),
        path: "/learned".into(),
        instructions: String::new(),
    });
    c.trigger.mode = DreamTriggerMode::Manual;
    c.budget.max_cost_usd_per_day = None;
    c
}

#[cfg(feature = "local")]
#[tokio::test]
async fn legacy_cutover_refuses_to_drop_dreaming_state() {
    use everruns::hosting::spi::HostBackends;
    use everruns::hosting::{AgentBuild, SessionHosting, SqliteHosting};

    for configured in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let memory = directory.path().join("memory");
        std::fs::create_dir(&memory).unwrap();
        let backends = HostBackends::in_memory();
        let mut builder = Agent::builder()
            .instructions("Acknowledge")
            .model(Model::simulated("Acknowledged"))
            .memory_directory("/memory/user", memory)
            .backends(backends.clone());
        if configured {
            builder = builder.capability(config());
        }
        let legacy = builder.build().unwrap();
        let session = Engine::new().create(legacy.clone());
        session.send_and_wait("Remember UTC.").await.unwrap();
        let id = session.session_id();
        if !configured {
            let record = backends
                .session_store
                .get_session(id)
                .await
                .unwrap()
                .unwrap();
            let org = everruns_core::host::in_process_internal_org_id(&record.organization_id);
            // A removed capability must not make its retained cursor, claim or
            // proposal disappear during a transfer to another authority.
            assert!(
                backends
                    .dreaming_store
                    .compare_exchange(org, &format!("session:{id}"), None, "{}", Some(id), None,)
                    .await
                    .unwrap()
            );
        }
        let profile = SqliteHosting::open(directory.path().join("authority.db")).unwrap();
        let build = AgentBuild::new("dreaming-cutover", "v1");
        let engine = Engine::builder()
            .hosting(profile.clone())
            .register(
                build.clone(),
                Agent::builder()
                    .instructions("Acknowledge")
                    .model(Model::simulated("Acknowledged"))
                    .build()
                    .unwrap(),
            )
            .build();
        let error = engine
            .cut_over_legacy_session(id, &legacy, &build)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("dreaming"), "{error}");
        assert!(profile.status(id).await.is_err());
        engine.abandon_legacy_cutover(id, &legacy).await.unwrap();
        session
            .send_and_wait("Continue on the original host.")
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn manual_dreaming_uses_engine_and_does_not_rewrite_conversation() {
    let dir = tempfile::tempdir().unwrap();
    let agent = Agent::builder()
        .instructions("Remember useful preferences")
        .model(Model::simulated("{\"notes\":[]}"))
        .memory_directory("/memory/user", dir.path())
        .capability(config())
        .build()
        .unwrap();
    let engine = Engine::new();
    let session = engine.create(agent);
    session
        .send_and_wait("Remember concise Rust examples.")
        .await
        .unwrap();
    let before = session.inspect().await.unwrap();
    assert!(before.tools.iter().any(|tool| tool.name == "read_file"));
    assert!(
        before
            .tools
            .iter()
            .any(|tool| tool.name == "list_directory")
    );
    assert!(engine.dream(session.session_id()).await.unwrap());
    let after = session.inspect().await.unwrap();
    assert_eq!(
        format!("{:?}", before.messages),
        format!("{:?}", after.messages)
    );
    assert_eq!(before.instructions, after.instructions);
    assert!(!engine.dream(session.session_id()).await.unwrap());
    assert!(!dir.path().join("learned").exists());
}
#[tokio::test]
async fn ordinary_workspace_paths_are_not_implicitly_authorized() {
    let agent = Agent::builder()
        .instructions("Remember")
        .model(Model::simulated("{\"notes\":[]}"))
        .capability(config())
        .build()
        .unwrap();
    let engine = Engine::new();
    let session = engine.create(agent);
    session.send_and_wait("Remember UTC.").await.unwrap();
    let error = engine.dream(session.session_id()).await.unwrap_err();
    assert!(error.to_string().contains("authorized writable Memory"));
}
#[tokio::test]
async fn unpriced_model_refuses_a_configured_dollar_budget() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = config();
    c.budget.max_cost_usd_per_day = Some(1.0);
    let agent = Agent::builder()
        .instructions("Remember")
        .model(Model::simulated("{\"notes\":[]}"))
        .memory_directory("/memory/user", dir.path())
        .capability(c)
        .build()
        .unwrap();
    let engine = Engine::new();
    let session = engine.create(agent);
    session.send_and_wait("Remember UTC.").await.unwrap();
    let error = engine.dream(session.session_id()).await.unwrap_err();
    assert!(error.to_string().contains("pricing"));
}

#[cfg(feature = "local")]
#[tokio::test]
async fn completed_evidence_cursor_survives_sqlite_engine_reconstruction() {
    use everruns::hosting::{AgentBuild, SqliteHosting};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("authority.sqlite");
    let memory = directory.path().join("memory");
    std::fs::create_dir(&memory).unwrap();
    let agent = Agent::builder()
        .instructions("Remember useful preferences")
        .model(Model::simulated("{\"notes\":[]}"))
        .memory_directory("/memory/user", &memory)
        .capability(config())
        .build()
        .unwrap();
    let build = AgentBuild::new("dreaming-test", "v1");
    let id = everruns::SessionId::new();
    {
        let engine = Engine::builder()
            .hosting(SqliteHosting::open(&path).unwrap())
            .register(build.clone(), agent.clone())
            .build();
        let session = engine.create_registered(id, &build).await.unwrap();
        session
            .send_and_wait("Remember UTC timestamps.")
            .await
            .unwrap();
        assert!(engine.dream(id).await.unwrap());
    }
    let engine = Engine::builder()
        .hosting(SqliteHosting::open(&path).unwrap())
        .register(build, agent)
        .build();
    let session = engine.resume(id).await.unwrap();
    let before = session.inspect().await.unwrap();
    assert!(!engine.dream(id).await.unwrap());
    assert_eq!(
        format!("{:?}", before.messages),
        format!("{:?}", session.inspect().await.unwrap().messages)
    );
}

#[tokio::test]
async fn sourced_notes_are_written_to_the_mounted_memory_surface() {
    let directory = tempfile::tempdir().unwrap();
    let message = everruns::MessageId::new();
    let output=serde_json::json!({"notes":[{"text":"Use UTC timestamps.","kind":"preference","evidence":[message.to_string()]}]}).to_string();
    let agent = Agent::builder()
        .instructions("Acknowledge")
        .model(Model::simulated(output))
        .memory_directory("/memory/user", directory.path())
        .capability(config())
        .build()
        .unwrap();
    let engine = Engine::new();
    let session = engine.create(agent);
    session
        .send_with_id(message, "Remember UTC timestamps.")
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();
    let instructions = session.inspect().await.unwrap().instructions;
    assert!(engine.dream(session.session_id()).await.unwrap());
    assert_eq!(instructions, session.inspect().await.unwrap().instructions);
    let files = std::fs::read_dir(directory.path().join("learned"))
        .unwrap()
        .collect::<std::io::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(files.len(), 1);
    let saved = std::fs::read_to_string(files[0].path()).unwrap();
    assert!(saved.contains("Use UTC timestamps."));
    assert!(saved.contains(&message.to_string()));
    // A fresh session must reach the same artifact through ordinary file tools,
    // not merely through the maintenance writer's filesystem port.
    let path = format!(
        "/memory/user/learned/{}",
        files[0].file_name().to_string_lossy()
    );
    let reader = Agent::builder()
        .instructions("Read saved Memory")
        .model(Model::simulated_with_config(
            everruns::LlmSimConfig::scripted(vec![
                everruns_llmsim::SimTurn::ToolCalls(vec![everruns_llmsim::SimToolCall {
                    name: "read_file".into(),
                    arguments: serde_json::json!({"path":path}),
                    id: Some("recall_note".into()),
                }]),
                everruns_llmsim::SimTurn::Assistant("Read complete.".into()),
            ]),
        ))
        .memory_directory("/memory/user", directory.path())
        .capability(config())
        .build()
        .unwrap();
    let recall = engine.create(reader);
    assert_eq!(
        recall
            .send_and_wait("Read the saved note.")
            .await
            .unwrap()
            .tool_calls,
        1
    );
    let history = recall.history().page().await.unwrap();
    let result = history
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .find_map(|part| match part {
            everruns::ContentPart::ToolResult(result) if result.tool_call_id == "recall_note" => {
                Some(result)
            }
            _ => None,
        })
        .expect("recorded Memory read");
    assert!(result.error.is_none(), "{result:?}");
    assert!(
        serde_json::to_string(&result.result)
            .unwrap()
            .contains("Use UTC timestamps.")
    );
    assert!(!engine.dream(session.session_id()).await.unwrap());
    assert_eq!(
        std::fs::read_dir(directory.path().join("learned"))
            .unwrap()
            .count(),
        1
    );
}

#[tokio::test]
async fn session_command_wakes_embedded_dreaming_without_starting_a_turn() {
    let directory = tempfile::tempdir().unwrap();
    let message = everruns::MessageId::new();
    let output = serde_json::json!({"notes":[{"text":"Use UTC timestamps.","kind":"preference","evidence":[message.to_string()]}]}).to_string();
    let agent = Agent::builder()
        .instructions("Acknowledge")
        .model(Model::simulated(output))
        .memory_directory("/memory/user", directory.path())
        .capability(config())
        .build()
        .unwrap();
    let engine = Engine::new();
    let session = engine.create(agent);
    session
        .send_with_id(message, "Remember UTC timestamps.")
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();
    let before = session.inspect().await.unwrap();
    let result = session.run_command("dream", None).await.unwrap();
    assert!(result.success, "{}", result.message);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Ok(entries) = std::fs::read_dir(directory.path().join("learned")) {
                let files = entries.collect::<std::io::Result<Vec<_>>>().unwrap();
                if files.len() == 1
                    && std::fs::read_to_string(files[0].path())
                        .unwrap()
                        .contains("Use UTC timestamps.")
                {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let after = session.inspect().await.unwrap();
    assert_eq!(
        format!("{:?}", before.messages),
        format!("{:?}", after.messages)
    );
    assert_eq!(before.instructions, after.instructions);
}
