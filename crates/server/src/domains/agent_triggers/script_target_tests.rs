//! Triggers that run a saved script: the script must exist, only schedule and
//! webhook triggers take one, and updates keep or clear it.

use super::*;
use crate::domains::agent_scripts::commands::CreateAgentScript;
use crate::domains::agent_scripts::types::CreateAgentScriptRequest;
use everruns_contracts::runtime::saved_scripts::ScriptRun;

#[tokio::test]
async fn a_trigger_can_target_an_existing_saved_script() {
    let db = Arc::new(StorageBackend::test_database());
    let (agent_id, _) = seed_agent(&db).await;
    let ctx = role_ctx(db, OrgRole::Owner);
    let run = |script: &str| ScriptRun {
        script: script.to_string(),
        input: None,
        wake_agent_on_failure: false,
    };
    let with_script = |script: ScriptRun| CreateAgentTriggerRequest {
        script: Some(script),
        ..webhook_req(false)
    };

    let error = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: with_script(run("triage")),
    }
    .run(&ctx)
    .await
    .expect_err("the script must exist");
    assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);

    CreateAgentScript {
        agent_id: agent_id.clone(),
        req: CreateAgentScriptRequest {
            name: "triage".to_string(),
            description: "Label new PRs.".to_string(),
            input_schema: None,
            body: "echo ok".to_string(),
        },
    }
    .run(&ctx)
    .await
    .expect("create script");

    let mut bad_input = run("triage");
    bad_input.input = Some(serde_json::json!([1]));
    let error = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: with_script(bad_input),
    }
    .run(&ctx)
    .await
    .expect_err("input must be an object");
    assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);

    let error = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: CreateAgentTriggerRequest {
            trigger_type: AgentTriggerType::GitHub,
            ..with_script(run("triage"))
        },
    }
    .run(&ctx)
    .await
    .expect_err("only schedule and webhook triggers run scripts");
    assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);

    let trigger = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: with_script(run("triage")),
    }
    .run(&ctx)
    .await
    .expect("a trigger can target an existing script");
    assert_eq!(
        trigger.config.get("script"),
        Some(&serde_json::to_value(run("triage")).unwrap())
    );

    // An update that leaves the script out keeps it; an empty name clears it.
    let kept = UpdateAgentTriggerCmd {
        agent_id: agent_id.clone(),
        trigger_id: trigger.id.to_string(),
        req: UpdateAgentTriggerRequest {
            message: Some("Run triage".to_string()),
            ..Default::default()
        },
    }
    .run(&ctx)
    .await
    .expect("update message");
    assert_eq!(
        kept.config.get("script"),
        Some(&serde_json::to_value(run("triage")).unwrap())
    );
    let cleared = UpdateAgentTriggerCmd {
        agent_id,
        trigger_id: trigger.id.to_string(),
        req: UpdateAgentTriggerRequest {
            script: Some(run("")),
            ..Default::default()
        },
    }
    .run(&ctx)
    .await
    .expect("clear script");
    assert_eq!(cleared.config.get("script"), None);
}
