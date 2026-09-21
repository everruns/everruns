//! Wire conversions for the Slack endpoint action seam (EVE-1024).
//!
//! The native `slack` capability runs in the worker; the endpoint's `bot_token`
//! lives in the control plane and stays there. So the *action* is what crosses
//! this boundary, and this module is the only place that knows both shapes.
//!
//! Errors cross as a closed [`proto::SlackActionErrorKind`] rather than a
//! message string: the capability renders "this session did not come from
//! Slack" as a tool error the model can act on and a transient fault as an
//! internal error, and telling those apart must not depend on parsing prose.

use everruns_platform::slack_action::{SlackAction, SlackActionError, SlackActionOutcome};

use crate::proto;

impl From<SlackAction> for proto::invoke_slack_action_request::Action {
    fn from(action: SlackAction) -> Self {
        match action {
            SlackAction::AddReaction {
                channel,
                timestamp,
                name,
            } => Self::AddReaction(proto::SlackAddReaction {
                channel,
                timestamp,
                name,
            }),
            SlackAction::UpdateMessage {
                channel,
                timestamp,
                text,
            } => Self::UpdateMessage(proto::SlackUpdateMessage {
                channel,
                timestamp,
                text,
            }),
            SlackAction::LookupUser { user_id } => {
                Self::LookupUser(proto::SlackLookupUser { user_id })
            }
            SlackAction::UploadFile {
                channel,
                thread_ts,
                filename,
                content,
                initial_comment,
            } => Self::UploadFile(proto::SlackUploadFile {
                channel,
                thread_ts,
                filename,
                content,
                initial_comment,
            }),
        }
    }
}

impl From<proto::invoke_slack_action_request::Action> for SlackAction {
    fn from(action: proto::invoke_slack_action_request::Action) -> Self {
        use proto::invoke_slack_action_request::Action;
        match action {
            Action::AddReaction(a) => Self::AddReaction {
                channel: a.channel,
                timestamp: a.timestamp,
                name: a.name,
            },
            Action::UpdateMessage(a) => Self::UpdateMessage {
                channel: a.channel,
                timestamp: a.timestamp,
                text: a.text,
            },
            Action::LookupUser(a) => Self::LookupUser { user_id: a.user_id },
            Action::UploadFile(a) => Self::UploadFile {
                channel: a.channel,
                thread_ts: a.thread_ts,
                filename: a.filename,
                content: a.content,
                initial_comment: a.initial_comment,
            },
        }
    }
}

impl From<SlackActionOutcome> for proto::invoke_slack_action_response::Result {
    fn from(outcome: SlackActionOutcome) -> Self {
        match outcome {
            SlackActionOutcome::ReactionAdded { already_reacted } => {
                Self::ReactionAdded(proto::SlackReactionAdded { already_reacted })
            }
            SlackActionOutcome::MessageUpdated { channel, timestamp } => {
                Self::MessageUpdated(proto::SlackMessageUpdated { channel, timestamp })
            }
            SlackActionOutcome::User {
                user_id,
                display_name,
                real_name,
                is_bot,
                tz,
            } => Self::User(proto::SlackUser {
                user_id,
                display_name,
                real_name,
                is_bot,
                tz,
            }),
            SlackActionOutcome::FileUploaded { file_id, permalink } => {
                Self::FileUploaded(proto::SlackFileUploaded { file_id, permalink })
            }
        }
    }
}

impl From<SlackActionError> for proto::SlackActionError {
    fn from(error: SlackActionError) -> Self {
        use proto::SlackActionErrorKind as Kind;
        // Only the variants that carry detail beyond their kind send a message;
        // the others render from the kind alone on the far side, so the wire
        // does not become the source of that wording.
        match error {
            SlackActionError::NoSlackSession => Self {
                kind: Kind::NoSlackSession as i32,
                message: None,
                retry_after_secs: None,
            },
            SlackActionError::EndpointUnavailable => Self {
                kind: Kind::EndpointUnavailable as i32,
                message: None,
                retry_after_secs: None,
            },
            SlackActionError::NotConfigured => Self {
                kind: Kind::NotConfigured as i32,
                message: None,
                retry_after_secs: None,
            },
            SlackActionError::Rejected(message) => Self {
                kind: Kind::Rejected as i32,
                message: Some(message),
                retry_after_secs: None,
            },
            SlackActionError::RateLimited { retry_after_secs } => Self {
                kind: Kind::RateLimited as i32,
                message: None,
                retry_after_secs,
            },
            SlackActionError::InvalidArgument(message) => Self {
                kind: Kind::InvalidArgument as i32,
                message: Some(message),
                retry_after_secs: None,
            },
            SlackActionError::Transient(message) => Self {
                kind: Kind::Transient as i32,
                message: Some(message),
                retry_after_secs: None,
            },
        }
    }
}

impl From<proto::SlackActionError> for SlackActionError {
    fn from(error: proto::SlackActionError) -> Self {
        use proto::SlackActionErrorKind as Kind;
        let detail = || {
            error
                .message
                .clone()
                .unwrap_or_else(|| "Slack action failed".to_string())
        };
        match Kind::try_from(error.kind) {
            Ok(Kind::NoSlackSession) => Self::NoSlackSession,
            Ok(Kind::EndpointUnavailable) => Self::EndpointUnavailable,
            Ok(Kind::NotConfigured) => Self::NotConfigured,
            Ok(Kind::Rejected) => Self::Rejected(detail()),
            Ok(Kind::RateLimited) => Self::RateLimited {
                retry_after_secs: error.retry_after_secs,
            },
            Ok(Kind::InvalidArgument) => Self::InvalidArgument(detail()),
            Ok(Kind::Transient) => Self::Transient(detail()),
            // An unset or unrecognised kind comes from a control plane newer
            // than this worker. Transient is the safe reading: it is reported
            // as an internal fault rather than told to the model as a fact
            // about its Slack session.
            Ok(Kind::Unspecified) | Err(_) => Self::Transient(detail()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip_action(action: SlackAction) {
        let wire: proto::invoke_slack_action_request::Action = action.clone().into();
        let back: SlackAction = wire.into();
        assert_eq!(back, action);
    }

    #[test]
    fn every_action_round_trips() {
        round_trip_action(SlackAction::AddReaction {
            channel: "C1".to_string(),
            timestamp: "1.2".to_string(),
            name: "eyes".to_string(),
        });
        round_trip_action(SlackAction::UpdateMessage {
            channel: "C1".to_string(),
            timestamp: "1.2".to_string(),
            text: "done".to_string(),
        });
        round_trip_action(SlackAction::LookupUser {
            user_id: "U1".to_string(),
        });
        round_trip_action(SlackAction::UploadFile {
            channel: "C1".to_string(),
            thread_ts: Some("1.2".to_string()),
            filename: "report.md".to_string(),
            content: b"hello".to_vec(),
            initial_comment: Some("here".to_string()),
        });
        // The absent-optional shape is the one a `None` mapping gets wrong.
        round_trip_action(SlackAction::UploadFile {
            channel: "C1".to_string(),
            thread_ts: None,
            filename: "report.md".to_string(),
            content: b"hello".to_vec(),
            initial_comment: None,
        });
    }

    fn round_trip_error(error: SlackActionError) {
        let wire: proto::SlackActionError = error.clone().into();
        let back: SlackActionError = wire.into();
        assert_eq!(
            std::mem::discriminant(&back),
            std::mem::discriminant(&error),
            "error kind changed across the wire: {error:?} -> {back:?}"
        );
        assert_eq!(
            back.is_tool_error(),
            error.is_tool_error(),
            "tool-vs-internal classification changed across the wire for {error:?}"
        );
    }

    #[test]
    fn every_error_keeps_its_kind_and_classification() {
        round_trip_error(SlackActionError::NoSlackSession);
        round_trip_error(SlackActionError::EndpointUnavailable);
        round_trip_error(SlackActionError::NotConfigured);
        round_trip_error(SlackActionError::Rejected("bad scope".to_string()));
        round_trip_error(SlackActionError::RateLimited {
            retry_after_secs: Some(30),
        });
        round_trip_error(SlackActionError::InvalidArgument("empty".to_string()));
        round_trip_error(SlackActionError::Transient("timeout".to_string()));
    }

    #[test]
    fn rate_limit_advice_survives() {
        let wire: proto::SlackActionError = SlackActionError::RateLimited {
            retry_after_secs: Some(42),
        }
        .into();
        let back: SlackActionError = wire.into();
        assert!(matches!(
            back,
            SlackActionError::RateLimited {
                retry_after_secs: Some(42)
            }
        ));
    }

    #[test]
    fn an_unknown_error_kind_is_read_as_transient() {
        // A control plane newer than this worker sends a kind we do not know.
        // Reading it as a fact about the Slack session would tell the model
        // something false, so it must land on the internal-fault side.
        let wire = proto::SlackActionError {
            kind: 9999,
            message: Some("from the future".to_string()),
            retry_after_secs: None,
        };
        let back: SlackActionError = wire.into();
        assert!(matches!(back, SlackActionError::Transient(_)));
        assert!(!back.is_tool_error());
    }

    #[test]
    fn every_outcome_maps_to_its_own_variant() {
        use proto::invoke_slack_action_response::Result as Wire;
        assert!(matches!(
            Wire::from(SlackActionOutcome::ReactionAdded {
                already_reacted: true
            }),
            Wire::ReactionAdded(proto::SlackReactionAdded {
                already_reacted: true
            })
        ));
        assert!(matches!(
            Wire::from(SlackActionOutcome::MessageUpdated {
                channel: "C1".to_string(),
                timestamp: "1.2".to_string(),
            }),
            Wire::MessageUpdated(_)
        ));
        assert!(matches!(
            Wire::from(SlackActionOutcome::User {
                user_id: "U1".to_string(),
                display_name: None,
                real_name: None,
                is_bot: false,
                tz: None,
            }),
            Wire::User(_)
        ));
        assert!(matches!(
            Wire::from(SlackActionOutcome::FileUploaded {
                file_id: "F1".to_string(),
                permalink: None,
            }),
            Wire::FileUploaded(_)
        ));
    }
}
