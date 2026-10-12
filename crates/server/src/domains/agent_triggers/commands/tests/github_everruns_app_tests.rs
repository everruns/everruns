// GitHub triggers on an agent that acts as the shared Everruns App (D3): the
// trigger can be created, and the shared App's deliveries reach it only for
// the installation the agent chose.

use super::*;
use crate::domains::agent_triggers::github::{GitHubDeliveryRecipients, dispatch_github_delivery};
use crate::domains::virtual_users::lifecycle::ensure_identity_for_agent;
use crate::github_apps::{EVERRUNS_APP_CONNECTION_TYPE, GITHUB_PROVIDER};
use crate::storage::{CreateOrganizationConnectionRow, CreateVirtualUserConnectionRow};

const CHOSEN: i64 = 9300;
const OTHER: i64 = 9301;

#[tokio::test]
async fn shared_app_deliveries_reach_agents_that_chose_the_installation() {
    let db = Arc::new(StorageBackend::test_database());
    let (agent_id, _) = seed_agent(&db).await;
    let ctx = role_ctx(db.clone(), OrgRole::Owner);
    for installation in [CHOSEN, OTHER] {
        db.create_organization_connection(CreateOrganizationConnectionRow {
            org_id: DEFAULT_ORG_ID,
            name: format!("acct-{installation}"),
            provider: GITHUB_PROVIDER.to_string(),
            connection_type: Some(EVERRUNS_APP_CONNECTION_TYPE.to_string()),
            installation_id: Some(installation),
            ..Default::default()
        })
        .await
        .unwrap();
    }
    let agent = db
        .get_agent_by_public_id(DEFAULT_ORG_ID, &agent_id)
        .await
        .unwrap()
        .unwrap();
    let (identity_id, _) = ensure_identity_for_agent(&db, DEFAULT_ORG_ID, &agent)
        .await
        .unwrap();
    db.upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
        virtual_user_id: identity_id,
        provider: GITHUB_PROVIDER.to_string(),
        connection_type: EVERRUNS_APP_CONNECTION_TYPE.to_string(),
        provider_user_id: None,
        provider_username: Some("acme".to_string()),
        access_token_encrypted: None,
        refresh_token_encrypted: None,
        scopes: None,
        expires_at: None,
        installation_id: Some(CHOSEN),
        provider_metadata: None,
    })
    .await
    .unwrap();

    // The shared App's webhook delivers to this trigger, so it may be created
    // without an App of the agent's own.
    CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: github_req(None),
    }
    .run(&ctx)
    .await
    .expect("create GitHub trigger on an everruns_app agent");

    let runner = Arc::new(RecordingRunner::default());
    let session_service = SessionService::new(db.clone());
    let message_service =
        MessageService::new(db.clone(), runner.clone(), EventDelivery::in_memory());
    let fire = async |installation: i64, delivery| {
        let recipients = GitHubDeliveryRecipients::everruns_app(
            &db,
            DEFAULT_ORG_ID,
            installation,
            Some("everruns"),
        )
        .await
        .unwrap();
        dispatch_github_delivery(
            &db,
            &session_service,
            &message_service,
            &recipients,
            &delivery,
            None,
        )
        .await
        .expect("delivery routed")
        .into_iter()
        .map(|dispatch| dispatch.outcome.expect("dispatch ok"))
        .collect::<Vec<_>>()
    };

    let opened = fire(
        CHOSEN,
        github_delivery("e1", "pull_request", "opened", "acme/api", "octo"),
    )
    .await;
    assert!(
        matches!(
            opened.as_slice(),
            [events::TriggerEventOutcome::Dispatched(_)]
        ),
        "got {opened:?}"
    );
    // Another installation the org links, which this agent did not choose.
    assert!(
        fire(
            OTHER,
            github_delivery("e2", "pull_request", "opened", "acme/api", "octo"),
        )
        .await
        .is_empty()
    );
    // The shared bot's own activity never wakes the agent.
    assert!(
        fire(
            CHOSEN,
            github_delivery("e3", "pull_request", "opened", "acme/api", "everruns[bot]"),
        )
        .await
        .is_empty()
    );
}
