//! Projects successful turns into the Agent or conversation view. Retried event
//! deliveries and runs finishing out of order cannot replace a newer snapshot.

use super::views::{AgentViewDefinition, AgentViewLayout, validate_layout};
use crate::records::FeatureFlagPolicy;
use crate::storage::StorageBackend;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use everruns_capabilities::capabilities::published_views::{PUBLISH_VIEW_TOOL, PUBLISHED_VIEWS_ID};
use everruns_contracts::typed_id::{AgentId, SessionId};
use everruns_core::{Event, EventListener, events::EventData, message::MessageOriginKind};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

pub(crate) const LISTENER: &str = "AgentViewPublication";
pub(crate) const EVENT_TYPES: &[&str] = &[
    "turn.started",
    "tool.completed",
    "turn.completed",
    "turn.failed",
    "turn.cancelled",
];

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Publication {
    data: Map<String, Value>,
    layout: Option<AgentViewLayout>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PublishedAgentView {
    pub revision: i64,
    pub title: String,
    pub layout: AgentViewLayout,
    pub data: Map<String, Value>,
    pub allow_personalization: bool,
    pub updated_at: Option<DateTime<Utc>>,
    pub personal_layout: Option<AgentViewLayout>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct AgentViewPreference {
    pub revision: i64,
    /// Pass null to restore the published layout.
    #[serde(deserialize_with = "Deserialize::deserialize")]
    #[schema(required = true)]
    pub layout: Option<AgentViewLayout>,
}

pub(crate) async fn read(
    db: &StorageBackend,
    org_id: i64,
    agent: AgentId,
    session: Option<SessionId>,
    viewer: Option<(&str, Uuid)>,
    agent_scope: bool,
) -> anyhow::Result<Option<PublishedAgentView>> {
    let Some((revision, definition, publication, updated_at)) = db
        .read_published_agent_view(org_id, agent, session, agent_scope)
        .await?
    else {
        return Ok(None);
    };
    let definition: AgentViewDefinition = serde_json::from_value(definition)?;
    let publication: Option<Publication> = publication.map(serde_json::from_value).transpose()?;
    let (data, layout) = publication
        .map(|publication| (publication.data, publication.layout))
        .unwrap_or_default();
    let personal_layout = match viewer {
        Some((audience, viewer)) if definition.allow_personalization => db
            .read_agent_view_preference(agent, audience, viewer, revision)
            .await?
            .map(serde_json::from_value)
            .transpose()?,
        _ => None,
    };
    Ok(Some(PublishedAgentView {
        revision,
        title: definition.title,
        layout: layout.unwrap_or(definition.layout),
        allow_personalization: definition.allow_personalization,
        updated_at,
        data,
        personal_layout,
    }))
}

pub(crate) async fn save_preference(
    db: &StorageBackend,
    agent: AgentId,
    audience: &str,
    viewer: Uuid,
    preference: AgentViewPreference,
) -> Result<(), crate::domains::common::CommandError> {
    use crate::domains::common::CommandError;
    if let Some(layout) = &preference.layout {
        validate_layout(layout)?;
    }
    let layout = preference
        .layout
        .map(serde_json::to_value)
        .transpose()
        .map_err(anyhow::Error::from)?;
    if !db
        .save_agent_view_preference(agent, audience, viewer, preference.revision, layout)
        .await?
    {
        return Err(CommandError::conflict(
            "Personalization is unavailable for this view revision.",
        ));
    }
    Ok(())
}

mod session_commands {
    use super::*;
    use crate::domains::agents::AGENT_VIEW;
    use crate::domains::common::*;
    use crate::domains::sessions::{GetSession, SESSION_VIEW};

    #[derive(Debug, Deserialize, Serialize, ToSchema, utoipa::IntoParams)]
    #[into_params(parameter_in = Query)]
    pub struct ViewReference {
        /// An explicit Agent view opened in Platform Chat.
        pub view_agent: Option<AgentId>,
    }

    struct Target {
        agent: AgentId,
        session: Option<SessionId>,
        agent_scope: bool,
    }

    async fn authorized_target(
        ctx: &Ctx,
        session: SessionId,
        reference: ViewReference,
    ) -> Result<Option<Target>, CommandError> {
        GetSession {
            session_id: session.to_string(),
        }
        .run(ctx)
        .await?;
        let may_view_agent = AGENT_VIEW
            .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
            .is_ok();
        if let Some(agent) = reference.view_agent {
            if !may_view_agent {
                return Err(CommandError::not_found("View"));
            }
            return Ok(ctx
                .db
                .get_agent_by_public_id(ctx.org_id(), &agent.to_string())
                .await?
                .map(|row| Target {
                    agent: row.id,
                    session: None,
                    agent_scope: true,
                }));
        }
        Ok(ctx
            .db
            .get_session(ctx.org_id(), session)
            .await?
            .and_then(|row| row.agent_id)
            .map(|agent| Target {
                agent,
                session: Some(session),
                agent_scope: may_view_agent,
            }))
    }

    #[derive(Debug, Deserialize, Serialize, ToSchema)]
    pub struct GetSessionAgentView {
        pub session_id: SessionId,
        #[serde(flatten)]
        pub reference: ViewReference,
    }

    #[command(
        name = "get_session_agent_view",
        category = "agent_views",
        summary = "Read a published view from Platform Chat",
        description = "Read the published Agent view for a visible session or an authorized Agent reference.",
        method = "GET",
        path = "/v1/sessions/{session_id}/agent-view",
        policy = SESSION_VIEW,
        http = plain,
        params(ViewReference),
    )]
    impl Command for GetSessionAgentView {
        type Output = Option<PublishedAgentView>;

        async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
            let Some(target) = authorized_target(ctx, self.session_id, self.reference).await?
            else {
                return Ok(None);
            };
            Ok(read(
                &ctx.db,
                ctx.org_id(),
                target.agent,
                target.session,
                ctx.caller.user_id.map(|id| ("platform", id)),
                target.agent_scope,
            )
            .await?)
        }
    }

    #[derive(Debug, Deserialize, Serialize, ToSchema)]
    pub struct SetSessionAgentViewPreference {
        pub session_id: SessionId,
        #[serde(flatten)]
        pub reference: ViewReference,
        #[serde(flatten)]
        pub preference: AgentViewPreference,
    }

    #[command(
        name = "set_session_agent_view_preference",
        category = "agent_views",
        description = "Save a private layout for your view of this Agent.",
        method = "PUT",
        path = "/v1/sessions/{session_id}/agent-view",
        policy = SESSION_VIEW,
        http = plain,
        params(ViewReference),
        request_body(AgentViewPreference),
    )]
    impl Command for SetSessionAgentViewPreference {
        type Output = Option<PublishedAgentView>;

        async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
            let target = authorized_target(ctx, self.session_id, self.reference)
                .await?
                .ok_or_else(|| CommandError::not_found("View"))?;
            let viewer = ctx
                .caller
                .user_id
                .ok_or_else(|| CommandError::forbidden("Sign in to save a layout"))?;
            save_preference(&ctx.db, target.agent, "platform", viewer, self.preference).await?;
            Ok(read(
                &ctx.db,
                ctx.org_id(),
                target.agent,
                target.session,
                Some(("platform", viewer)),
                target.agent_scope,
            )
            .await?)
        }
    }
}

pub use session_commands::{GetSessionAgentView, SetSessionAgentViewPreference};

fn validate_publication(definition: Value, draft: Value) -> Option<Value> {
    let definition: AgentViewDefinition = serde_json::from_value(definition).ok()?;
    let publication: Publication = serde_json::from_value(draft).ok()?;
    if serde_json::to_vec(&publication.data).ok()?.len() > 16 * 1024
        || publication
            .data
            .keys()
            .any(|key| !definition.data_fields.contains(key))
    {
        return None;
    }
    if let Some(layout) = &publication.layout
        && (!definition.allow_agent_layout || validate_layout(layout).is_err())
    {
        return None;
    }
    serde_json::to_value(publication).ok()
}

pub(crate) struct AgentViewPublication {
    db: Arc<StorageBackend>,
    policy: FeatureFlagPolicy,
}

impl AgentViewPublication {
    pub fn new(db: Arc<StorageBackend>, policy: FeatureFlagPolicy) -> Self {
        Self { db, policy }
    }

    async fn enabled(&self, org_id: i64) -> anyhow::Result<bool> {
        Ok(
            crate::services::org_feature_flags::resolve_org_feature_flags(
                &self.db,
                org_id,
                &self.policy,
            )
            .await?
            .agent_views,
        )
    }

    async fn project(&self, event: &Event) -> anyhow::Result<()> {
        let session = event.session_id;
        match &event.data {
            EventData::TurnStarted(started) => {
                let Some(agent) = started.agent_id else {
                    return Ok(());
                };
                let Some(org) = self.db.session_org_id(session).await? else {
                    return Ok(());
                };
                if !self.db.has_agent_views(org).await? || !self.enabled(org).await? {
                    return Ok(());
                }
                // THREAT[TM-TENANT-024]: only the server attaches a Trigger origin.
                // A channel consumer updates only their conversation.
                let agent_scope = started
                    .origin
                    .as_ref()
                    .is_some_and(|origin| origin.kind == MessageOriginKind::AgentTrigger);
                self.db
                    .start_agent_view_turn(session, started.turn_id, agent, agent_scope, event.ts)
                    .await?;
            }
            EventData::ToolCompleted(tool)
                if tool.success
                    && tool.tool_name == PUBLISH_VIEW_TOOL
                    && tool.capability_id.as_deref() == Some(PUBLISHED_VIEWS_ID) =>
            {
                let Some(turn) = event.context.turn_id else {
                    return Ok(());
                };
                let Some(sequence) = event.sequence else {
                    return Ok(());
                };
                let result = tool
                    .result
                    .as_ref()
                    .and_then(|parts| parts.iter().find_map(|part| part.as_text()));
                if let Some(draft) = result
                    .and_then(|text| serde_json::from_str::<Value>(text).ok())
                    .and_then(|mut result| result.get_mut("view_update").map(Value::take))
                {
                    self.db
                        .stage_agent_view(session, turn, sequence, draft)
                        .await?;
                }
            }
            EventData::TurnCompleted(done) => {
                let enabled = match self.db.session_org_id(session).await? {
                    Some(org) => self.db.has_agent_views(org).await? && self.enabled(org).await?,
                    None => false,
                };
                self.db
                    .finish_agent_view_turn(session, done.turn_id, |settings, draft| {
                        enabled
                            .then(|| validate_publication(settings, draft))
                            .flatten()
                    })
                    .await?;
            }
            _ if matches!(event.event_type.as_str(), "turn.failed" | "turn.cancelled") => {
                if let Some(turn) = event.context.turn_id {
                    self.db
                        .finish_agent_view_turn(session, turn, |_, _| None)
                        .await?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[async_trait]
impl EventListener for AgentViewPublication {
    async fn on_event(&self, event: &Event) {
        if let Err(error) = self.project(event).await {
            tracing::warn!(%error, session_id = %event.session_id, "Could not project an Agent view");
        }
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(EVENT_TYPES.to_vec())
    }

    fn name(&self) -> &'static str {
        LISTENER
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::CreateSessionRow;
    use everruns_contracts::typed_id::{MessageId, PrincipalId, TurnId};
    use everruns_core::{
        DEFAULT_ORG_ID, DeploymentGrade, EventContext, ToolCompletedData, TurnCompletedData,
        TurnStartedData, message::ContentPart,
    };
    use serde_json::json;

    async fn stage(
        projector: &AgentViewPublication,
        session: SessionId,
        agent: AgentId,
        started_at: DateTime<Utc>,
        scheduled: bool,
        value: &str,
    ) -> Event {
        let turn_id = TurnId::new();
        let input_message_id = MessageId::new();
        let context = EventContext::turn(turn_id, input_message_id);
        let mut start = Event::new(
            session,
            context.clone(),
            TurnStartedData {
                turn_id,
                input_message_id,
                agent_id: Some(agent),
                input_content: None,
                agent_name: None,
                agent_description: None,
                origin: scheduled.then(|| {
                    everruns_core::message::MessageOrigin::new(MessageOriginKind::AgentTrigger)
                }),
            },
        );
        start.ts = started_at;
        projector.project(&start).await.unwrap();
        let mut result = Event::new(
            session,
            context.clone(),
            ToolCompletedData::success(
                "call-one".into(),
                PUBLISH_VIEW_TOOL.into(),
                vec![ContentPart::tool_result_text(&json!({
                    "view_update":{"data":{"status":value}},
                }))],
                None,
            )
            .with_capability_attribution(Some(PUBLISHED_VIEWS_ID.into()), None),
        );
        result.sequence = Some(2);
        projector.project(&result).await.unwrap();
        Event::new(
            session,
            context,
            TurnCompletedData {
                turn_id,
                ..Default::default()
            },
        )
    }

    #[tokio::test]
    async fn publishes_only_completed_turns_and_keeps_scopes_and_run_order() {
        let db = Arc::new(StorageBackend::test_database());
        let org = DEFAULT_ORG_ID;
        let agent = AgentId::from_uuid(db.create_test_agent(org, Uuid::now_v7()).await);
        db.replace_org_feature_flags(
            org,
            &std::collections::HashMap::from([("agent_views".into(), true)]),
        )
        .await
        .unwrap();
        db.replace_agent_view_settings(
            org,
            agent,
            0,
            Some(json!({
                "title":"Sweep", "layout":{"format":"html","source":"<p>{{status}}</p>"},
                "data_fields":["status"], "allow_personalization":true
            })),
        )
        .await
        .unwrap();
        let first = db
            .create_session(CreateSessionRow {
                org_id: org,
                agent_id: Some(agent),
                owner_principal_id: PrincipalId::from_seed(1),
                ..Default::default()
            })
            .await
            .unwrap()
            .id;
        let second = db
            .create_session(CreateSessionRow {
                org_id: org,
                agent_id: Some(agent),
                owner_principal_id: PrincipalId::from_seed(1),
                ..Default::default()
            })
            .await
            .unwrap()
            .id;
        let projector = AgentViewPublication::new(
            db.clone(),
            FeatureFlagPolicy::from_env(DeploymentGrade::Prod),
        );
        let now = Utc::now();
        let older = stage(
            &projector,
            first,
            agent,
            now - chrono::Duration::seconds(1),
            true,
            "old",
        )
        .await;
        let latest = stage(&projector, second, agent, now, true, "new").await;
        let before = read(&db, org, agent, None, None, true)
            .await
            .unwrap()
            .unwrap();
        assert!(before.data.is_empty());
        projector.project(&latest).await.unwrap();
        projector.project(&older).await.unwrap();
        projector.project(&latest).await.unwrap();
        let local = stage(&projector, first, agent, now, false, "thread").await;
        projector.project(&local).await.unwrap();

        let global = read(&db, org, agent, None, None, true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(global.data["status"], "new");
        let first_only = read(&db, org, agent, Some(first), None, false)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first_only.data["status"], "thread");
        let second_only = read(&db, org, agent, Some(second), None, false)
            .await
            .unwrap()
            .unwrap();
        assert!(second_only.data.is_empty());

        let alice = Uuid::now_v7();
        let bob = Uuid::now_v7();
        save_preference(
            &db,
            agent,
            "platform",
            alice,
            AgentViewPreference {
                revision: global.revision,
                layout: Some(AgentViewLayout::Html {
                    source: "<p>My layout</p>".into(),
                }),
            },
        )
        .await
        .unwrap();
        let alice_view = read(&db, org, agent, None, Some(("platform", alice)), true)
            .await
            .unwrap()
            .unwrap();
        assert!(alice_view.personal_layout.is_some());
        for viewer in [("platform", bob), ("work_chat:test", alice)] {
            let other = read(&db, org, agent, None, Some(viewer), true)
                .await
                .unwrap()
                .unwrap();
            assert!(other.personal_layout.is_none());
        }
    }
}
