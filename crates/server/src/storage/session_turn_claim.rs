#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReserveActiveTurnSlotResult {
    /// Message accepted. New turns are marked active; parked turns are claimed
    /// for exclusive resolution before the caller writes their replacement.
    Accepted {
        previous_status: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimWaitingTurnResult {
    Claimed,
    Conflict { current_status: String },
    SessionNotFound,
}
