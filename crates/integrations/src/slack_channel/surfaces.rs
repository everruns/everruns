//! Compatibility names and typed error mapping; provider surfaces live in Channelkit.
use super::web_api::SlackApiError;
use everruns_contracts::runtime::channel::DeliveryResult as ChannelDeliveryResult;
pub const SLACK_RECIPIENT_USER_ID: &str = "slack_recipient_user_id";
pub const SLACK_RECIPIENT_TEAM_ID: &str = "slack_recipient_team_id";
pub fn classify_slack_failure(error: SlackApiError) -> ChannelDeliveryResult {
    match error {
        SlackApiError::Permanent(message) => ChannelDeliveryResult::PermanentError(message),
        SlackApiError::Delivery(outcome) => outcome,
        other => ChannelDeliveryResult::TransientError(other.to_string()),
    }
}
