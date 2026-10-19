// Agent script storage round-trips: create -> get -> list -> partial update ->
// soft delete, the unique active (agent, name) rule, and org scoping.

use super::*;
use serde_json::json;

fn new_row(agent_id: AgentId, name: &str) -> CreateAgentScriptRow {
    CreateAgentScriptRow {
        org_id: DEFAULT_ORG_ID,
        id: everruns_contracts::typed_id::ScriptId::new(),
        agent_id,
        name: name.to_string(),
        description: "d".to_string(),
        input_schema: Some(json!({"type": "object"})),
        body: "echo 1".to_string(),
    }
}

async fn agent(db: &StorageBackend) -> AgentId {
    AgentId::from_uuid(db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await)
}

#[tokio::test]
async fn test_agent_script_create_get_list_update_delete_round_trip() {
    let db = StorageBackend::test_database();
    let agent_id = agent(&db).await;

    let created = db
        .create_agent_script(new_row(agent_id, "hello"))
        .await
        .unwrap();
    assert_eq!(created.status, "active");
    assert_eq!(created.agent_id, agent_id);
    assert_eq!(created.input_schema, Some(json!({"type": "object"})));

    let fetched = db
        .get_agent_script(DEFAULT_ORG_ID, created.id)
        .await
        .unwrap()
        .expect("script exists");
    assert_eq!(fetched.body, "echo 1");
    assert!(
        db.get_agent_script(999, created.id)
            .await
            .unwrap()
            .is_none(),
        "other orgs cannot read it"
    );

    // A partial update leaves the other fields alone.
    let updated = db
        .update_agent_script(
            DEFAULT_ORG_ID,
            created.id,
            UpdateAgentScript {
                body: Some("echo 2".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .expect("update returns the row");
    assert_eq!(updated.body, "echo 2");
    assert_eq!(updated.description, "d");
    assert_eq!(updated.input_schema, Some(json!({"type": "object"})));

    assert!(
        db.delete_agent_script(DEFAULT_ORG_ID, created.id)
            .await
            .unwrap()
    );
    assert!(
        !db.delete_agent_script(DEFAULT_ORG_ID, created.id)
            .await
            .unwrap(),
        "a second delete finds nothing active"
    );
    assert!(
        db.list_agent_scripts(DEFAULT_ORG_ID, agent_id, false)
            .await
            .unwrap()
            .is_empty()
    );
    let with_archived = db
        .list_agent_scripts(DEFAULT_ORG_ID, agent_id, true)
        .await
        .unwrap();
    assert_eq!(with_archived.len(), 1);
    assert_eq!(with_archived[0].status, "archived");

    // Archived rows are read-only.
    assert!(
        db.update_agent_script(DEFAULT_ORG_ID, created.id, UpdateAgentScript::default())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn test_agent_script_name_is_unique_among_active_scripts_of_one_agent() {
    let db = StorageBackend::test_database();
    let (a, b) = (agent(&db).await, agent(&db).await);

    let first = db.create_agent_script(new_row(a, "hello")).await.unwrap();
    assert!(
        db.create_agent_script(new_row(a, "hello")).await.is_err(),
        "duplicate active name on one agent"
    );
    db.create_agent_script(new_row(b, "hello"))
        .await
        .expect("another agent may reuse the name");

    db.delete_agent_script(DEFAULT_ORG_ID, first.id)
        .await
        .unwrap();
    db.create_agent_script(new_row(a, "hello"))
        .await
        .expect("an archived script frees its name");

    assert_eq!(
        db.list_agent_scripts(DEFAULT_ORG_ID, a, false)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db.list_agent_scripts(DEFAULT_ORG_ID, b, false)
            .await
            .unwrap()
            .len(),
        1
    );
}
