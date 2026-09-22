use chrono::{DateTime, Utc};
use everruns_core::events::EventRequest;
use uuid::Uuid;

const WAITING_TURN_CLAIM_LEASE_SECS: i64 = 60;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReserveActiveTurnSlotResult {
    /// Message accepted. New turns are marked active; parked turns are claimed
    /// for exclusive resolution before the caller writes their replacement.
    Accepted {
        previous_status: String,
        resolution_claim: Option<WaitingTurnResolutionClaim>,
    },
    Conflict {
        current_status: String,
    },
    AtCapacity {
        active_turns: i64,
    },
    SessionNotFound,
}

pub const RESOLVING_TOOL_RESULTS_STATUS: &str = "resolving_tool_results";
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WaitingTurnResolutionPlan {
    pub kind: String,
    pub events: Vec<EventRequest>,
    pub session_values: Vec<WaitingTurnSessionValue>,
    pub response: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WaitingTurnSessionValue {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingTurnResolutionClaim {
    pub resolution_id: Uuid,
    pub claim_token: Uuid,
    pub plan: WaitingTurnResolutionPlan,
    pub recovered: bool,
}

impl PartialEq for WaitingTurnResolutionPlan {
    fn eq(&self, other: &Self) -> bool {
        serde_json::to_value(self).ok() == serde_json::to_value(other).ok()
    }
}

impl Eq for WaitingTurnResolutionPlan {}

pub fn waiting_turn_claim_lease_expires_at() -> DateTime<Utc> {
    Utc::now() + chrono::Duration::seconds(WAITING_TURN_CLAIM_LEASE_SECS)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimWaitingTurnResult {
    Claimed(WaitingTurnResolutionClaim),
    Conflict { current_status: String },
    SessionNotFound,
}
