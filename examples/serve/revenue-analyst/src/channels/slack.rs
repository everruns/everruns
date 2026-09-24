use serve::prelude::*;

/// The team's Slack. Mentions start (or continue, per thread) a session, and
/// replies go back to the thread. Without `SLACK_BOT_TOKEN` replies are
/// printed instead of posted.
#[channel]
pub fn slack() -> Slack {
    Slack::from_secrets().mention_only()
}
