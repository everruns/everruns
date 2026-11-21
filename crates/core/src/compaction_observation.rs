//! Read-only operational assessments shared by hosted and embedded observers.
//! An execution verdict never establishes semantic preservation of history.

use serde::{Deserialize, Serialize};

use crate::events::EventData;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// A bounded, provenance-preserving event excerpt for a host-owned assessment.
pub struct CompactionEvidenceExcerpt {
    /// Canonical event identifier, suitable for evidence citations.
    pub id: String,
    /// Absolute journal sequence within the session.
    pub sequence: i32,
    /// Original event type; content keeps its original authority.
    pub event_type: String,
    /// Serialized data preview, possibly incomplete JSON.
    pub text: String,
    /// Whether the preview omits any serialized bytes.
    pub truncated: bool,
}

/// Serialize into a byte-limited preview without first allocating a complete
/// tool result. A partial JSON excerpt is explicitly marked as truncated.
pub fn bounded_json_preview(value: &impl Serialize, max_bytes: usize) -> (String, bool) {
    struct Limited {
        bytes: Vec<u8>,
        limit: usize,
        truncated: bool,
    }
    impl std::io::Write for Limited {
        fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
            let take = input.len().min(self.limit.saturating_sub(self.bytes.len()));
            self.bytes.extend_from_slice(&input[..take]);
            if take < input.len() {
                self.truncated = true;
                return Err(std::io::Error::from(std::io::ErrorKind::WriteZero));
            }
            Ok(input.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Limited {
        bytes: Vec::with_capacity(max_bytes.min(2048)),
        limit: max_bytes,
        truncated: false,
    };
    if serde_json::to_writer(&mut writer, value).is_err() {
        writer.truncated = true;
    }
    while std::str::from_utf8(&writer.bytes).is_err() {
        writer.bytes.pop();
    }
    (
        String::from_utf8(writer.bytes).unwrap_or_default(),
        writer.truncated,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Outcome of one bounded check, rather than a whole-session quality claim.
pub enum CompactionVerdict {
    /// The available evidence supports this check.
    Passed,
    /// The available evidence contradicts this check.
    Failed,
    /// Evidence or permission is insufficient to assess this check.
    InsufficientEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Operational assessment derived from a terminal runtime event.
pub struct CompactionObservation {
    /// Outcome for the reported operation, independent of semantic fidelity.
    pub verdict: CompactionVerdict,
    /// Content-free explanation of the operational outcome.
    pub reason: String,
    /// Reported checkpoint identity, when installed.
    pub checkpoint_id: Option<String>,
    /// Reported journal coverage boundary, when available.
    pub source_sequence: Option<i64>,
    /// Whether the runtime explicitly allowed raw-history evidence access.
    pub continuity_eligible: bool,
    /// Whether this operation represents a continuity boundary rather than masking.
    pub continuity_applicable: bool,
}

/// Assess the runtime's terminal event without model calls or side effects.
/// Message counts are not a reduction metric: masking and native replay can
/// retain the same count. Missing provider measurements stay unknown.
pub fn observe_compaction(data: &EventData) -> Option<CompactionObservation> {
    use CompactionVerdict::*;
    let continuity_applicable = matches!(data, EventData::ContextCompacted(event) if event.checkpoint_id.is_some() || event.strategy_used.contains("summar") || event.strategy_used.contains("native") || event.strategy_used == "anthropic_server_compaction");
    let (verdict, reason, checkpoint_id, source_sequence, continuity_eligible) = match data {
        EventData::ContextCompactionFailed(event) => (
            Failed,
            format!("Compaction failed during {:?}.", event.stage),
            event.checkpoint_id.clone(),
            event.source_sequence,
            false,
        ),
        EventData::ContextCompactionSkipped(event) => (
            InsufficientEvidence,
            format!("No checkpoint was installed: {:?}.", event.skip_reason),
            None,
            event.source_sequence,
            false,
        ),
        EventData::ContextCompacted(event) => {
            let reduced = event
                .tokens_before
                .zip(event.tokens_after)
                .is_some_and(|(a, b)| b < a)
                || event
                    .bytes_before
                    .zip(event.bytes_after)
                    .is_some_and(|(a, b)| b < a);
            let installed = event.checkpoint_id.is_some();
            let eligible = event.history_evidence_allowed
                && (installed
                    || event.strategy_used.contains("summar")
                    || event.strategy_used.contains("native")
                    || event.strategy_used == "anthropic_server_compaction");
            let (verdict, reason) = if event.source_sequence.is_some_and(|sequence| sequence < 0) {
                (Failed, "The reported coverage boundary is invalid.")
            } else if installed {
                (
                    Passed,
                    "The runtime reported a checkpoint installation; continuity is assessed separately.",
                )
            } else if reduced {
                (
                    Passed,
                    "The runtime reported a smaller request representation; continuity is assessed separately.",
                )
            } else {
                (
                    InsufficientEvidence,
                    "Compaction completed without comparable size measurements or an installed checkpoint.",
                )
            };
            (
                verdict,
                reason.to_string(),
                event.checkpoint_id.clone(),
                event.source_sequence,
                eligible,
            )
        }
        _ => return None,
    };
    Some(CompactionObservation {
        verdict,
        reason,
        checkpoint_id,
        source_sequence,
        continuity_eligible,
        continuity_applicable,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::events::deserialize_event_data;
    use serde_json::json;

    fn compacted(overrides: serde_json::Value) -> EventData {
        let mut value = json!({"strategy_used":"native","messages_before":316,"messages_after":316,"duration_ms":5,"steps":[],"model":"test","checkpoint_id":"checkpoint","source_sequence":30});
        value
            .as_object_mut()
            .unwrap()
            .extend(overrides.as_object().unwrap().clone());
        deserialize_event_data("context.compacted", value)
    }

    #[test]
    fn native_installation_does_not_require_smaller_message_counts_or_visible_tokens() {
        let observed =
            observe_compaction(&compacted(json!({"history_evidence_allowed":true}))).unwrap();
        assert_eq!(observed.verdict, CompactionVerdict::Passed);
        assert!(observed.continuity_eligible);
    }

    #[test]
    fn old_events_and_runtime_private_history_are_not_semantic_evidence() {
        assert!(
            !observe_compaction(&compacted(json!({})))
                .unwrap()
                .continuity_eligible
        );
        assert!(
            !observe_compaction(&compacted(json!({"history_evidence_allowed":false})))
                .unwrap()
                .continuity_eligible
        );
    }

    #[test]
    fn request_local_masking_is_not_a_durable_semantic_checkpoint() {
        let observed = observe_compaction(&compacted(json!({"checkpoint_id":null,"strategy_used":"observation_masking","tokens_before":90000,"tokens_after":51000,"history_evidence_allowed":true}))).unwrap();
        assert_eq!(observed.verdict, CompactionVerdict::Passed);
        assert!(!observed.continuity_eligible);
        assert!(
            observe_compaction(&compacted(json!({"checkpoint_id":null})))
                .is_some_and(|event| event.verdict == CompactionVerdict::InsufficientEvidence)
        );
    }

    #[test]
    fn previews_stop_at_a_byte_budget_and_keep_valid_unicode() {
        let (preview, truncated) = bounded_json_preview(&json!({"text":"世🙂".repeat(100000)}), 51);
        assert!(truncated);
        assert!(preview.len() <= 51);
        assert!(std::str::from_utf8(preview.as_bytes()).is_ok());
    }
}
