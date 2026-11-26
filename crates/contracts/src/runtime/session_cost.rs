//! Narrow, portable inputs for the session Cost read. Provider request and
//! response payloads never cross the data-plane link for this read.
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct CostGenerationRow {
    pub turn_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub actual_cost_usd: Option<f64>,
    pub estimated_cost_usd: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct CostTurnRow {
    pub turn_no: i32,
    pub turn_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub status: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct CostTurnOriginRow {
    /// Public turn id (`turn_...`).
    pub turn_id: String,
    /// `schedule`, `agent_trigger`, ...; `None` for a message a person sent.
    pub kind: Option<String>,
    /// The schedule or trigger behind it, when there is one.
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct CostToolRow {
    pub name: String,
    pub calls: i64,
    pub failed: i64,
    pub duration_ms: i64,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct SessionCostInputs {
    pub generations: Vec<CostGenerationRow>,
    pub origins: Vec<CostTurnOriginRow>,
}

pub fn cost_inputs(events: &[crate::runtime::events::Event]) -> SessionCostInputs {
    use crate::message_origin::MessageOrigin;
    use crate::runtime::events::EventData;
    let mut inputs = SessionCostInputs::default();
    let mut origins = std::collections::HashMap::new();
    for event in events {
        if let EventData::InputMessage(data) = &event.data {
            origins.insert(
                data.message.id,
                MessageOrigin::from_message_metadata(data.message.metadata.as_ref()),
            );
        }
    }
    for event in events {
        match &event.data {
            EventData::LlmGeneration(data) => {
                if let Some(call) = super::session_spend::spend_call(
                    event.context.turn_id.map(|id| id.uuid()),
                    &data.metadata,
                ) {
                    inputs.generations.push(CostGenerationRow {
                        turn_id: call.turn_id,
                        created_at: event.ts,
                        provider: call.provider,
                        model: call.model,
                        input_tokens: call.input_tokens,
                        output_tokens: call.output_tokens,
                        cache_read_tokens: call.cache_read_tokens,
                        cache_creation_tokens: call.cache_creation_tokens,
                        actual_cost_usd: call.actual_cost_usd,
                        estimated_cost_usd: call.estimated_cost_usd,
                    });
                }
            }
            EventData::TurnStarted(data) => {
                let origin = data
                    .origin
                    .as_ref()
                    .or_else(|| origins.get(&data.input_message_id).and_then(|o| o.as_ref()));
                inputs.origins.push(CostTurnOriginRow {
                    turn_id: data.turn_id.to_string(),
                    kind: origin.map(|o| o.kind.as_str().to_owned()),
                    source_id: origin.and_then(|o| o.source_id.clone()),
                });
            }
            _ => {}
        }
    }
    inputs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::events::{
        Event, EventContext, InputMessageData, LlmGenerationData, TokenUsage, TurnStartedData,
    };
    use crate::runtime::message::RuntimeMessage;
    use crate::typed_id::{SessionId, TurnId};

    #[test]
    fn investigation_cost_projection_keeps_origin_and_usage_without_request_payloads() {
        let session = SessionId::new();
        let turn = TurnId::new();
        let mut message = RuntimeMessage::user("private prompt text");
        crate::message_origin::MessageOrigin::schedule("sched_test", "Check build")
            .write(&mut message.metadata);
        let input_id = message.id;
        let events = vec![
            Event::new(
                session,
                EventContext::empty(),
                InputMessageData::new(message),
            ),
            Event::new(
                session,
                EventContext::empty(),
                TurnStartedData {
                    turn_id: turn,
                    input_message_id: input_id,
                    input_content: None,
                    agent_id: None,
                    agent_name: None,
                    agent_description: None,
                    origin: None,
                },
            ),
            Event::new(
                session,
                EventContext {
                    turn_id: Some(turn),
                    ..EventContext::empty()
                },
                LlmGenerationData::success(
                    vec![],
                    vec![],
                    None,
                    vec![],
                    "model".into(),
                    Some("openrouter".into()),
                    Some(TokenUsage::new(10, 5)),
                    None,
                    None,
                ),
            ),
        ];
        let input = cost_inputs(&events);
        assert_eq!(input.generations.len(), 1);
        assert_eq!(input.generations[0].input_tokens, 10);
        assert_eq!(input.origins[0].source_id.as_deref(), Some("sched_test"));
        let wire = serde_json::to_string(&input).unwrap();
        assert!(!wire.contains("private prompt text"));
        assert!(!wire.contains("request"));
    }
}
