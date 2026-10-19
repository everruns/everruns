use serve::prelude::*;

/// The team's Slack. Mentions start (or continue, per thread) a session, and
/// replies go back to the thread. Without `SLACK_BOT_TOKEN` replies are
/// printed instead of posted; without `SLACK_SIGNING_SECRET` unsigned
/// requests are accepted, for trying it locally (`start` requires both).
#[channel]
pub fn slack() -> Slack {
    Slack::from_env().mention_only()
}
