//! The host services an org's own decision model runs on, for Slack's
//! relevance check. The check itself is shared with rooms:
//! [`crate::services::relevance`].

/// What an org with `system_decisions: organization` decides with.
pub type SlackOrgDecisions = crate::services::relevance::OrgDecisionHost;
