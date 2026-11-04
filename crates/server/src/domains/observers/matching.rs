// Which scores a trace slice gets: shared by the live listener and backfill.
//
// Decisions:
// - Sampling for turn and tool scopes is a fresh draw per slice and observer.
//   Session scope samples deterministically on (session, observer), so a
//   session that idles many times is either always scored or never, and its
//   one score is re-armed rather than drawn again.
// - A session score waits SESSION_SETTLE_SECONDS after the idle that armed
//   it; the next idle pushes that out again (see enqueue_trace_scores).

use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::{AgentId, HarnessId, TraceScoreId};
use tracing::error;
use uuid::Uuid;

use crate::domains::observers::record::{
    ObserverMatch, ObserverScope, ObserverScorerConfig, SESSION_SCORE_KEY, SESSION_SETTLE_SECONDS,
};
use crate::storage::{CreateTraceScoreRow, ObserverRow};

/// Tag marking synthetic eval-run sessions, which observers never score.
pub const EVAL_SESSION_TAG: &str = "eval";

/// The trace slice an event closes.
#[derive(Debug, Clone, PartialEq)]
pub enum Slice {
    /// A turn completed.
    Turn { turn_id: String },
    /// The session went idle.
    Session,
    /// A tool call completed.
    Tool { call_id: String, tool: String },
}

impl Slice {
    fn scope(&self) -> ObserverScope {
        match self {
            Slice::Turn { .. } => ObserverScope::Turn,
            Slice::Session => ObserverScope::Session,
            Slice::Tool { .. } => ObserverScope::Tool,
        }
    }

    fn key(&self) -> &str {
        match self {
            Slice::Turn { turn_id } => turn_id,
            Slice::Session => SESSION_SCORE_KEY,
            Slice::Tool { call_id, .. } => call_id,
        }
    }

    /// The slice a stored event closes, from its type and JSON data.
    pub fn from_event(event_type: &str, data: &serde_json::Value) -> Option<Self> {
        let text = |key: &str| data.get(key).and_then(|v| v.as_str()).map(str::to_string);
        match event_type {
            "turn.completed" => Some(Slice::Turn {
                turn_id: text("turn_id")?,
            }),
            "session.idled" => Some(Slice::Session),
            "tool.completed" => Some(Slice::Tool {
                call_id: text("tool_call_id")?,
                tool: text("tool_name")?,
            }),
            _ => None,
        }
    }
}

/// The session a slice belongs to, as matching needs it.
pub struct SessionFacts<'a> {
    pub session_id: Uuid,
    pub agent_id: Option<Uuid>,
    pub harness_id: Option<Uuid>,
    pub tags: &'a [String],
}

/// The scores `observers` want for `slice`. `sample` false scores every
/// matching slice (a backfill that asks for all of them).
pub fn plan_scores(
    observers: &[ObserverRow],
    session: &SessionFacts<'_>,
    slice: &Slice,
    sample: bool,
    now: DateTime<Utc>,
) -> Vec<CreateTraceScoreRow> {
    if session.tags.iter().any(|t| t == EVAL_SESSION_TAG) {
        return Vec::new();
    }
    let scope = slice.scope();
    let mut planned = Vec::new();
    for observer in observers {
        let match_config: ObserverMatch = match serde_json::from_value(
            observer.match_config.clone(),
        ) {
            Ok(m) => m,
            Err(e) => {
                error!(observer_id = %observer.public_id, error = %e, "Invalid observer match config");
                continue;
            }
        };
        if !match_config.matches(
            session.agent_id.map(AgentId::from_uuid),
            session.harness_id.map(HarnessId::from_uuid),
            session.tags,
        ) {
            continue;
        }
        let scorers: Vec<ObserverScorerConfig> = match serde_json::from_value(
            observer.scorers.clone(),
        ) {
            Ok(s) => s,
            Err(e) => {
                error!(observer_id = %observer.public_id, error = %e, "Invalid observer scorers");
                continue;
            }
        };
        let wanted: Vec<_> = scorers
            .into_iter()
            .filter(|scorer| scorer.scope == scope)
            .filter(|scorer| match slice {
                Slice::Tool { tool, .. } => scorer.grades_tool(tool),
                _ => true,
            })
            .collect();
        if wanted.is_empty() {
            continue;
        }
        let drawn = match slice {
            Slice::Session => stable_draw(session.session_id, observer.id),
            _ => rand::random::<f64>(),
        };
        if sample && !sampled(observer.sampling_rate, drawn) {
            continue;
        }
        let not_before = matches!(slice, Slice::Session)
            .then(|| now + chrono::Duration::seconds(SESSION_SETTLE_SECONDS));
        for scorer in wanted {
            planned.push(CreateTraceScoreRow {
                public_id: TraceScoreId::from_uuid(Uuid::now_v7()).to_string(),
                observer_id: observer.id,
                scorer_key: scorer.key,
                session_id: session.session_id,
                turn_id: slice.key().to_string(),
                agent_id: session.agent_id,
                harness_id: session.harness_id,
                scope: scope.as_str().to_string(),
                not_before,
            });
        }
    }
    planned
}

/// Whether a draw in [0, 1) falls inside `rate`. `rate >= 1.0` always
/// scores; `rate <= 0.0` never does.
pub fn sampled(rate: f64, drawn: f64) -> bool {
    if rate >= 1.0 {
        return true;
    }
    if rate <= 0.0 {
        return false;
    }
    drawn < rate
}

/// A draw in [0, 1) fixed by the session and observer.
fn stable_draw(session_id: Uuid, observer_id: Uuid) -> f64 {
    use sha2::{Digest, Sha256};
    let digest = Sha256::new()
        .chain_update(session_id.as_bytes())
        .chain_update(observer_id.as_bytes())
        .finalize();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    (u64::from_be_bytes(bytes) >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::evals::record::Scorer;
    use crate::domains::observers::record::ScorerMethod;

    fn observer(scorers: Vec<ObserverScorerConfig>, rate: f64) -> ObserverRow {
        ObserverRow {
            id: Uuid::now_v7(),
            org_id: 1,
            public_id: "observer_00000000000000000000000000000001".into(),
            name: "o".into(),
            description: None,
            match_config: serde_json::json!({}),
            sampling_rate: rate,
            scorers: serde_json::to_value(scorers).unwrap(),
            status: "active".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            alert: None,
        }
    }

    fn scorer(key: &str, scope: ObserverScope, tools: Option<Vec<&str>>) -> ObserverScorerConfig {
        ObserverScorerConfig {
            key: key.into(),
            scope,
            tools: tools.map(|t| t.into_iter().map(String::from).collect()),
            method: ScorerMethod::Rule {
                rule: Scorer::Contains {
                    text: "x".into(),
                    weight: 1.0,
                },
            },
        }
    }

    fn facts(tags: &[String]) -> SessionFacts<'_> {
        SessionFacts {
            session_id: Uuid::now_v7(),
            agent_id: None,
            harness_id: None,
            tags,
        }
    }

    #[test]
    fn each_slice_gets_only_its_scope() {
        let observers = vec![observer(
            vec![
                scorer("turn", ObserverScope::Turn, None),
                scorer("convo", ObserverScope::Session, None),
                scorer("lookup", ObserverScope::Tool, Some(vec!["lookup_order"])),
            ],
            1.0,
        )];
        let now = Utc::now();
        let session = facts(&[]);
        let turn = plan_scores(
            &observers,
            &session,
            &Slice::Turn {
                turn_id: "turn_1".into(),
            },
            true,
            now,
        );
        assert_eq!(turn.len(), 1);
        assert_eq!(turn[0].scorer_key, "turn");
        assert!(turn[0].not_before.is_none());

        let idle = plan_scores(&observers, &session, &Slice::Session, true, now);
        assert_eq!(idle.len(), 1);
        assert_eq!(idle[0].turn_id, SESSION_SCORE_KEY);
        assert_eq!(idle[0].scope, "session");
        assert_eq!(
            idle[0].not_before,
            Some(now + chrono::Duration::seconds(SESSION_SETTLE_SECONDS))
        );

        let tool = |name: &str| Slice::Tool {
            call_id: "call_1".into(),
            tool: name.into(),
        };
        let lookup = plan_scores(&observers, &session, &tool("lookup_order"), true, now);
        assert_eq!(lookup.len(), 1);
        assert_eq!(lookup[0].turn_id, "call_1");
        assert!(plan_scores(&observers, &session, &tool("bash"), true, now).is_empty());
    }

    #[test]
    fn eval_sessions_and_unsampled_slices_get_nothing() {
        let observers = vec![observer(vec![scorer("t", ObserverScope::Turn, None)], 0.0)];
        let turn = Slice::Turn {
            turn_id: "turn_1".into(),
        };
        let now = Utc::now();
        assert!(plan_scores(&observers, &facts(&[]), &turn, true, now).is_empty());
        // A backfill asking for every slice ignores the rate...
        assert_eq!(
            plan_scores(&observers, &facts(&[]), &turn, false, now).len(),
            1
        );
        // ...but never scores an eval session.
        let eval = ["eval".to_string()];
        assert!(plan_scores(&observers, &facts(&eval), &turn, false, now).is_empty());
    }

    #[test]
    fn session_sampling_is_stable() {
        let session = Uuid::now_v7();
        let observer = Uuid::now_v7();
        assert_eq!(
            stable_draw(session, observer),
            stable_draw(session, observer)
        );
        let draws: Vec<f64> = (0..200)
            .map(|_| stable_draw(Uuid::now_v7(), observer))
            .collect();
        assert!(draws.iter().all(|d| (0.0..1.0).contains(d)));
        let inside = draws.iter().filter(|d| sampled(0.5, **d)).count();
        assert!((50..150).contains(&inside), "{inside}");
    }

    #[test]
    fn slices_read_from_stored_events() {
        assert_eq!(
            Slice::from_event("turn.completed", &serde_json::json!({"turn_id": "turn_9"})),
            Some(Slice::Turn {
                turn_id: "turn_9".into()
            })
        );
        assert_eq!(
            Slice::from_event(
                "tool.completed",
                &serde_json::json!({"tool_call_id": "c", "tool_name": "bash"})
            ),
            Some(Slice::Tool {
                call_id: "c".into(),
                tool: "bash".into()
            })
        );
        assert_eq!(
            Slice::from_event("session.idled", &serde_json::json!({})),
            Some(Slice::Session)
        );
        assert_eq!(
            Slice::from_event("turn.started", &serde_json::json!({})),
            None
        );
    }
}
