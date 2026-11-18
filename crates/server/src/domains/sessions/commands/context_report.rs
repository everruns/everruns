//! Read-only context estimates and effective compaction metadata.
use super::*;
use everruns_contracts::model_profiles::get_model_profile;
use everruns_core::SessionContextReport;
use everruns_core::compaction_status::{CompactionEventSummary, CompactionStatus};

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetSessionContextReport {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

#[command(
    name = "get_session_context_report", category = "sessions",
    description = "Inspect the last main model request, effective compaction policy, and safe checkpoint metadata. Inspection never compacts history or calls a model.",
    method = "GET", path = "/v1/sessions/{session_id}/context-report",
    policy = super::super::SESSION_VIEW, positional = "session_id",
    cli = CliRoute::new(&["sessions"], "context").with_args(&[CliArg::new("session_id").at(1).long("session")]).with_examples(&[CliExample::new("See what is filling a session's context window", "everruns sessions context session_01h9")]),
)]
impl Command for GetSessionContextReport {
    type Output = SessionContextReport;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        let session = q::get_session(ctx, session_id, ctx.caller.user_id).await?;
        let configs = q::session_service(ctx)?
            .resolved_session_capability_configs(
                ctx.org_id(),
                Some(session.harness_id),
                session.agent_id,
                &session.capabilities,
            )
            .await?;
        let mut compaction =
            CompactionStatus::from_configs(&configs, ctx.capability_service.registry());
        let plane = crate::domains::data_plane::session_plane::SessionPlane::of(
            &ctx.db,
            ctx.org_id(),
            session.id,
        )
        .await?;
        let latest = if let Some(plane) = &plane {
            // Remote observations are bounded. During a large initial backfill
            // there may be no main-request sample in this recent window.
            let rows = plane
                .events(1, i32::MAX, &["llm.generation"], Some(128))
                .await?;
            rows.into_iter().rev().find_map(|row| {
                main_generation(&row.event_type, row.data)
                    .map(|data| (i64::from(row.sequence), data))
            })
        } else {
            ctx.db
                .database()
                .latest_context_generation(session_id)
                .await
                .map_err(anyhow::Error::from)?
                .and_then(|row| {
                    main_generation(&row.event_type, row.data)
                        .map(|data| (i64::from(row.sequence), data))
                })
        };
        let types = [
            "context.compacting",
            "context.compacted",
            "context.compaction.skipped",
            "context.compaction.failed",
        ];
        let observation = if let Some(plane) = &plane {
            plane
                .events(1, i32::MAX, &types, Some(1))
                .await?
                .pop()
                .map(|row| deserialize_event_data(&row.event_type, row.data))
        } else {
            ctx.db
                .list_events(
                    session_id,
                    None,
                    None,
                    &types.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                    &[],
                    None,
                    Some(1),
                )
                .await?
                .pop()
                .map(|row| deserialize_event_data(&row.event_type, row.data))
        };
        compaction.last_event = observation
            .as_ref()
            .and_then(CompactionEventSummary::from_event);
        if plane.is_none() {
            let store = ctx.encryption.as_ref().map(|encryption| {
                crate::storage::DbCompactionCheckpointStore::new(ctx.db.clone(), encryption.clone())
            });
            let model = latest.as_ref().and_then(|(_, data)| {
                data.metadata
                    .provider
                    .as_deref()
                    .map(|provider| (provider, data.metadata.model.as_str()))
            });
            compaction
                .inspect_checkpoint(
                    store
                        .as_ref()
                        .map(|s| s as &dyn everruns_core::CompactionCheckpointStore),
                    session_id,
                    model,
                )
                .await
                .map_err(anyhow::Error::from)?;
        }
        let mut report = if let Some((sequence, data)) = latest {
            let window = data
                .metadata
                .provider
                .as_deref()
                .and_then(parse_provider_type)
                .and_then(|provider| get_model_profile(&provider, &data.metadata.model))
                .and_then(|p| p.limits)
                .and_then(|limits| u32::try_from(limits.context).ok());
            let mut report = everruns_core::build_session_context_report_from_generation(
                session.id.to_string(),
                &data,
                window,
                session.usage,
            );
            report.as_of_sequence = Some(sequence);
            report
        } else {
            SessionContextReport {
                session_id: session.id.to_string(),
                model: "unknown".into(),
                measurement_available: false,
                as_of_sequence: None,
                compaction: None,
                context_window_tokens: None,
                estimated_input_tokens: 0,
                sections: vec![],
                contributions: vec![],
                cumulative_usage: session.usage,
            }
        };
        report.compaction = Some(compaction);
        Ok(report)
    }
}

fn main_generation(
    kind: &str,
    data: serde_json::Value,
) -> Option<everruns_core::events::LlmGenerationData> {
    let EventData::LlmGeneration(data) = deserialize_event_data(kind, data) else {
        return None;
    };
    let summary = data.metadata.request_options.as_ref().is_some_and(|o| {
        o.metadata
            .get("purpose")
            .is_some_and(|p| p == "continuity_summary")
    });
    (!summary).then_some(data)
}
