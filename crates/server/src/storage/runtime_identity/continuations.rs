//! A provider retry continues its own verified input; ordinary schedules are anonymous.
use super::*;
use crate::storage::SessionScheduleRow;
use chrono::{Duration, Utc};
use everruns_contracts::typed_id::MessageId;

type InvocationIdentity = (Option<VirtualUserId>, Option<Uuid>, Option<Uuid>);

#[derive(Clone, Copy)]
struct AttestedRetry {
    input_message_id: MessageId,
    identity: InvocationIdentity,
}

impl StorageBackend {
    pub(crate) async fn record_scheduled_runtime_invocation(
        &self,
        schedule: &SessionScheduleRow,
        message_id: Uuid,
        responder_agent_id: Option<Uuid>,
    ) -> Result<()> {
        if let Some(original) = self.attested_retry_identity(schedule).await? {
            self.validate_runtime_invocation_identity(
                schedule.org_id,
                original.identity.0,
                original.identity.1,
                original.identity.2,
                InvocationRows::default(),
            )
            .await?;
            if self
                .insert_retry_invocation_if_current(schedule, message_id, original)
                .await?
            {
                return Ok(());
            }
        }
        self.record_runtime_invocation(
            schedule.org_id,
            schedule.session_id,
            message_id,
            None,
            None,
            responder_agent_id,
        )
        .await
    }

    // The identity veto belongs to the final INSERT snapshot, after validation;
    // a separate precheck lets a different person arrive before authority is stored.
    async fn insert_retry_invocation_if_current(
        &self,
        schedule: &SessionScheduleRow,
        message_id: Uuid,
        original: AttestedRetry,
    ) -> Result<bool> {
        let result = sqlx::query(
            "INSERT INTO runtime_invocations (input_message_id,org_id,session_id,virtual_user_id,management_user_id,responder_agent_id) \
             SELECT $1,origin.org_id,origin.session_id,origin.virtual_user_id,origin.management_user_id,origin.responder_agent_id \
             FROM runtime_invocations origin WHERE origin.org_id=$2 AND origin.session_id=$3 AND origin.input_message_id=$4 \
             AND origin.virtual_user_id IS NOT DISTINCT FROM $5 AND origin.management_user_id IS NOT DISTINCT FROM $6 \
             AND origin.responder_agent_id IS NOT DISTINCT FROM $7 \
             AND EXISTS (SELECT 1 FROM (SELECT virtual_user_id,management_user_id,responder_agent_id FROM runtime_invocations \
                WHERE org_id=$2 AND session_id=$3 ORDER BY input_message_id DESC LIMIT 1) latest \
                WHERE latest.virtual_user_id IS NOT DISTINCT FROM origin.virtual_user_id \
                AND latest.management_user_id IS NOT DISTINCT FROM origin.management_user_id \
                AND latest.responder_agent_id IS NOT DISTINCT FROM origin.responder_agent_id) \
             ON CONFLICT (input_message_id) DO UPDATE SET input_message_id=EXCLUDED.input_message_id \
             WHERE runtime_invocations.org_id=EXCLUDED.org_id AND runtime_invocations.session_id=EXCLUDED.session_id \
             AND runtime_invocations.virtual_user_id IS NOT DISTINCT FROM EXCLUDED.virtual_user_id \
             AND runtime_invocations.management_user_id IS NOT DISTINCT FROM EXCLUDED.management_user_id \
             AND runtime_invocations.responder_agent_id IS NOT DISTINCT FROM EXCLUDED.responder_agent_id",
        )
        .bind(message_id)
        .bind(schedule.org_id)
        .bind(schedule.session_id)
        .bind(original.input_message_id.uuid())
        .bind(original.identity.0)
        .bind(original.identity.1)
        .bind(original.identity.2)
        .execute(self.database().pool()).await?;
        Ok(result.rows_affected() == 1)
    }

    async fn attested_retry_identity(
        &self,
        schedule: &SessionScheduleRow,
    ) -> Result<Option<AttestedRetry>> {
        let now = Utc::now();
        let Some(at) = schedule.scheduled_at else {
            return Ok(None);
        };
        if schedule.cron_expression.is_some()
            || !schedule.enabled
            || schedule.deleted_at.is_some()
            || schedule.trigger_count != 0
            || schedule.created_at < now - Duration::minutes(15)
            || at < schedule.created_at - Duration::seconds(30)
            || at > schedule.created_at + Duration::seconds(615)
        {
            return Ok(None);
        }

        // THREAT[TM-SCHED-008]: only a trusted failure event can bind a short retry
        // to its original invocation. Prompt text, timer ownership and user message
        // metadata cannot supply this proof. Bound history work before filtering.
        let context: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT context FROM (SELECT context,data,metadata,created_at FROM events \
             WHERE session_id=$1 AND event_type='reason.completed' \
             ORDER BY sequence DESC LIMIT 128) recent \
             WHERE created_at >= $3 AND data->>'success'='false' \
             AND metadata->'platform_auto_retry'->>'schedule_id'=$2 LIMIT 1",
        )
        .bind(schedule.session_id)
        .bind(schedule.id.to_string())
        .bind(schedule.created_at)
        .fetch_optional(self.database().pool())
        .await?;
        let source = context
            .as_ref()
            .and_then(|c| c.get("input_message_id"))
            .and_then(serde_json::Value::as_str)
            .and_then(|id| id.parse::<MessageId>().ok());
        let Some(source) = source else {
            return Ok(None);
        };
        let original: Option<InvocationIdentity> = sqlx::query_as(
            "SELECT virtual_user_id,management_user_id,responder_agent_id FROM runtime_invocations \
             WHERE org_id=$1 AND session_id=$2 AND input_message_id=$3",
        )
        .bind(schedule.org_id)
        .bind(schedule.session_id)
        .bind(source.uuid())
        .fetch_optional(self.database().pool())
        .await?;
        let Some(original) = original else {
            return Ok(None);
        };
        Ok(Some(AttestedRetry {
            input_message_id: source,
            identity: original,
        }))
    }
}

#[cfg(test)]
#[path = "continuations_tests.rs"]
mod tests;
