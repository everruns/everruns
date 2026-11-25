use super::*;

#[tokio::test]
async fn channel_actions_obey_the_agents_current_liveness() {
    let fixture = Fixture::new();
    let (_, channel_id, public_id) = fixture.seed_channel(ORG, "slack", "xoxb-secret").await;
    let channel = fixture
        .db
        .get_ingress_channel_by_public_id(&public_id)
        .await
        .unwrap()
        .unwrap();
    let agent_id = AgentId::from_uuid(channel.agent_id);
    let session_id = fixture
        .seed_session(ORG, Some(agent_id.uuid()), Some(channel_id), vec![])
        .await;
    let session = fixture
        .db
        .get_session(ORG, session_id)
        .await
        .unwrap()
        .unwrap();
    for (status, suspended) in [("archived", false), ("active", true)] {
        fixture
            .db
            .update_agent(
                ORG,
                agent_id,
                crate::storage::UpdateAgent {
                    status: Some(status.into()),
                    exposures_suspended: Some(suspended),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            fixture
                .invoker(ORG, session_id)
                .resolve_action_channel(&session)
                .await,
            Err(SlackActionError::ChannelUnavailable)
        ));
    }
}

#[tokio::test]
async fn a_session_cannot_borrow_another_agents_channel() {
    let fixture = Fixture::new();
    let (agent_id, _, _) = fixture.seed_channel(ORG, "slack", "xoxb-own").await;
    let (_, other_channel_id, other_public_id) =
        fixture.seed_channel(ORG, "slack", "xoxb-other").await;
    for (channel_fk, tags) in [
        (Some(other_channel_id), vec![]),
        (None, vec![format!("slack:channel:{other_public_id}")]),
    ] {
        let session_id = fixture
            .seed_session(ORG, Some(agent_id), channel_fk, tags)
            .await;
        let session = fixture
            .db
            .get_session(ORG, session_id)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            fixture
                .invoker(ORG, session_id)
                .resolve_action_channel(&session)
                .await,
            Err(SlackActionError::NoSlackSession)
        ));
    }
}
