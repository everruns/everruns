use super::super::{
    ObserverWorkerDeps,
    judge::{MonitoringAdmission, MonitoringNotAdmitted},
};
use super::{CONTINUITY_KEY, EXECUTION_KEY, evidence, policy};
use crate::storage::{CompleteTraceScoreRow, ObserverRow, TraceScoreRow};
use everruns_contracts::typed_id::SessionId;
use everruns_core::compaction_observation::{CompactionObservation, CompactionVerdict};
use sha2::{Digest, Sha256};

fn result(verdict: CompactionVerdict, reason: impl Into<String>) -> CompleteTraceScoreRow {
    let pass = match verdict {
        CompactionVerdict::Passed => Some(true),
        CompactionVerdict::Failed => Some(false),
        CompactionVerdict::InsufficientEvidence => None,
    };
    CompleteTraceScoreRow {
        status: "completed".into(),
        pass,
        value: pass.map(|pass| if pass { 1.0 } else { 0.0 }),
        label: Some(
            match verdict {
                CompactionVerdict::Passed => "passed",
                CompactionVerdict::Failed => "failed",
                CompactionVerdict::InsufficientEvidence => "insufficient_evidence",
            }
            .into(),
        ),
        reason: Some(reason.into()),
        ..Default::default()
    }
}

pub fn sampled(org: i64, event: &str, rate: f64) -> bool {
    let digest = Sha256::digest(format!("compaction-v1:{org}:{event}"));
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    rate >= 1.0 || (rate > 0.0 && (u64::from_be_bytes(bytes) as f64 / u64::MAX as f64) < rate)
}

pub async fn process_score(
    deps: &ObserverWorkerDeps,
    score: &TraceScoreRow,
    observer: &ObserverRow,
) -> anyhow::Result<()> {
    use CompactionVerdict::InsufficientEvidence as Unknown;
    let db = &deps.db;
    let current = policy(db, score.org_id).await?;
    let session_id = SessionId::from_uuid(score.session_id);
    let session = db.get_session_unscoped(session_id).await?;
    let metadata = score.evidence_ref.clone().unwrap_or_default();
    let permitted = observer.org_id == score.org_id
        && observer.status == "active"
        && score.scope == "compaction"
        && current.enabled
        && metadata.get("consent_version") == Some(&serde_json::to_value(observer.updated_at)?)
        && session
            .as_ref()
            .is_some_and(|session| session.org_id == score.org_id && current.allows(&session.tags));
    if !permitted {
        db.finish_monitoring_score(
            score,
            result(
                Unknown,
                "Monitoring consent or session eligibility changed; no content was assessed.",
            ),
        )
        .await?;
        return Ok(());
    }
    let observation: Option<CompactionObservation> = metadata
        .get("observation")
        .cloned()
        .map(serde_json::from_value)
        .transpose()?;
    let Some(observation) = observation else {
        db.finish_monitoring_score(
            score,
            result(Unknown, "Runtime compaction evidence is missing."),
        )
        .await?;
        return Ok(());
    };
    if score.scorer_key == EXECUTION_KEY {
        db.finish_monitoring_score(score, result(observation.verdict, observation.reason))
            .await?;
        return Ok(());
    }
    let continued = metadata
        .get("continuation_sequence")
        .and_then(|value| value.as_i64())
        .and_then(|value| i32::try_from(value).ok());
    let compacted = metadata
        .get("event_sequence")
        .and_then(|value| value.as_i64())
        .and_then(|value| i32::try_from(value).ok());
    let source = observation
        .source_sequence
        .and_then(|value| i32::try_from(value).ok());
    let Some((model, compacted, source, continued)) = current
        .model_id
        .zip(compacted)
        .zip(source)
        .zip(continued)
        .map(|(((model, compacted), source), continued)| (model, compacted, source, continued))
    else {
        db.finish_monitoring_score(
            score,
            result(
                Unknown,
                "A judge model, covered source boundary or following terminal turn is unavailable.",
            ),
        )
        .await?;
        return Ok(());
    };
    if score.scorer_key != CONTINUITY_KEY
        || !observation.continuity_eligible
        || current.daily_budget_micros() <= 0
        || !sampled(score.org_id, &score.turn_id, current.sampling_rate)
        || source < 1
        || compacted < source
        || continued <= compacted
    {
        db.finish_monitoring_score(
            score,
            result(
                Unknown,
                "This checkpoint is not eligible or selected for a paid continuity assessment.",
            ),
        )
        .await?;
        return Ok(());
    }
    let Some(judge) = deps.graders.compaction_judge() else {
        db.finish_monitoring_score(score, result(Unknown, "A monitoring judge is unavailable."))
            .await?;
        return Ok(());
    };
    let evidence = evidence::gather(db, session_id, source, compacted, continued).await?;
    if !evidence
        .excerpts
        .iter()
        .any(|excerpt| excerpt.sequence <= source)
        || !evidence
            .excerpts
            .iter()
            .any(|excerpt| excerpt.sequence > compacted)
    {
        db.finish_monitoring_score(
            score,
            result(
                Unknown,
                "The bounded sample does not contain both source and continuation evidence.",
            ),
        )
        .await?;
        return Ok(());
    }
    let call = judge
        .judge_compaction(
            score.org_id,
            model,
            evidence::SYSTEM_PROMPT.into(),
            serde_json::to_string(&evidence)?,
            MonitoringAdmission {
                score_id: score.id,
                attempt: score.attempts,
                policy: &current,
            },
        )
        .await;
    let completion = match call {
        Ok(Some(completion)) => completion,
        Ok(None) => {
            db.finish_monitoring_score(
                score,
                result(
                    Unknown,
                    "The selected judge cannot perform an admitted monitoring call.",
                ),
            )
            .await?;
            return Ok(());
        }
        Err(error) if error.is::<MonitoringNotAdmitted>() => {
            db.finish_monitoring_score(
                score,
                result(
                    Unknown,
                    "Consent, model pricing or monitoring budget did not admit this assessment.",
                ),
            )
            .await?;
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let mut completed = match evidence::parse(&completion.text, &evidence) {
        Ok(judgment) => result(
            judgment.verdict,
            format!(
                "{} Evidence: {}",
                judgment.reason,
                judgment.evidence_ids.join(", ")
            ),
        ),
        Err(_) => result(
            Unknown,
            "The judge did not return a valid, evidence-supported assessment.",
        ),
    };
    completed.judge_input_tokens = completion
        .input_tokens
        .map(|tokens| tokens.min(i64::MAX as u64) as i64);
    completed.judge_output_tokens = completion
        .output_tokens
        .map(|tokens| tokens.min(i64::MAX as u64) as i64);
    completed.judge_cost_usd = completion.cost_usd;
    db.finish_monitoring_score(score, completed).await?;
    Ok(())
}
