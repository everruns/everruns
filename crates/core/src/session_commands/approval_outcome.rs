//! Shared materialization of human decisions and unattended approval outcomes.
use super::ToolApprovalDecision;
use chrono::{DateTime, Utc};
use everruns_contracts::tool_approval_types::{
    StoredToolApproval, ToolApprovalRequired, always_decision_storage_key,
    one_off_decision_storage_key,
};
/// How a pending request ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOutcome {
    Decided(ToolApprovalDecision),
    /// Left out of a submission that answered its siblings.
    NotApproved,
    /// Nobody answered before the deadline.
    Expired,
}

impl ApprovalOutcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Decided(ToolApprovalDecision::Allow) => "allow",
            Self::Decided(ToolApprovalDecision::AllowAlways) => "allow_always",
            Self::Decided(ToolApprovalDecision::Reject) => "reject",
            Self::Decided(ToolApprovalDecision::RejectAlways) => "reject_always",
            Self::NotApproved => "not_approved",
            Self::Expired => "expired",
        }
    }

    pub fn allows(self) -> bool {
        matches!(
            self,
            Self::Decided(ToolApprovalDecision::Allow | ToolApprovalDecision::AllowAlways)
        )
    }

    /// What is recorded for the retried call to find. Silence records nothing:
    /// a retry asks again rather than inheriting a decision nobody made.
    pub fn stored_record(
        self,
        request: &ToolApprovalRequired,
        now: DateTime<Utc>,
    ) -> Option<(String, StoredToolApproval)> {
        let tool = request.tool.as_str();
        match self {
            Self::Decided(ToolApprovalDecision::Allow) => Some((
                one_off_decision_storage_key(&request.fingerprint),
                StoredToolApproval::one_off(tool, &request.fingerprint, true, now),
            )),
            Self::Decided(ToolApprovalDecision::Reject) => Some((
                one_off_decision_storage_key(&request.fingerprint),
                StoredToolApproval::one_off(tool, &request.fingerprint, false, now),
            )),
            Self::Decided(ToolApprovalDecision::AllowAlways) => Some((
                always_decision_storage_key(tool),
                StoredToolApproval::always(tool, true, now),
            )),
            Self::Decided(ToolApprovalDecision::RejectAlways) => Some((
                always_decision_storage_key(tool),
                StoredToolApproval::always(tool, false, now),
            )),
            Self::NotApproved | Self::Expired => None,
        }
    }

    /// The decision as the person would say it. The request is engine-authored,
    /// so nothing in the transcript claims the synthetic call and its result
    /// never reaches the model; this line is what does.
    pub fn spoken(self, tool: &str) -> String {
        match self {
            Self::Decided(ToolApprovalDecision::Allow) => format!(
                "I approved `{tool}` this once. Call it again now with exactly the same arguments."
            ),
            Self::Decided(ToolApprovalDecision::AllowAlways) => format!(
                "I approved `{tool}` for the rest of this session. Call it again now with the same arguments."
            ),
            Self::Decided(ToolApprovalDecision::Reject) => format!(
                "I rejected `{tool}`. Don't run it, and don't try to get the same result another way."
            ),
            Self::Decided(ToolApprovalDecision::RejectAlways) => {
                format!("I rejected `{tool}` for the rest of this session. Don't call it again.")
            }
            Self::NotApproved => {
                format!("I didn't approve `{tool}`, so it must not run.")
            }
            Self::Expired => format!(
                "I didn't answer the approval request for `{tool}` in time, so it is not approved. Don't run it."
            ),
        }
    }
}
