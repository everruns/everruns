//! Who sent a session's input messages, for the tools that act as that
//! person (data plane v2, step 11.6b): the platform tools (Platform Chat).
//!
//! On a v1 worker the control plane finds the person in its own record of
//! the turn (`runtime_invocations`), written when the message came through
//! its API. A v2 session's messages come through the edge to the shard, so
//! the control plane never sees them; the session's shard keeps the record.
//!
//! Decisions:
//! - THREAT[TM-AGENT-017]: the executor that runs a turn never names the
//!   person. It names the session and the turn's input message; the shard
//!   that forwards its lookup asks the session's shard who sent that message
//!   (`gateway_dp.rs`), and the control plane checks the person is still a
//!   member of the org, with their current role.
//!   An injected capability's session token names the person the same way:
//!   the executor that signs it reads the session's record (`act`), never
//!   anything the turn's tools pass (`mcp.rs`).
//! - A sender is recorded only by a trusted admission path: the gateway
//!   from the caller's verified org token (`sub`, a user), and the control
//!   plane for a message a person sent through it. A channel message, an
//!   agent trigger, a webhook have none, so their turns cannot act as a
//!   person, as on v1.
//! - A message the platform admits on the session's behalf (a schedule's
//!   run, a task wake) acts as the last person who wrote, as v1 copies the
//!   previous turn's record. One recorded before anyone wrote has none.
//! - The record keeps the latest [`KEPT`] messages: a person's turn is
//!   recent when its tools run. An older one acts as no one (fail closed).

use std::collections::VecDeque;

use everruns_contracts::typed_id::MessageId;
use serde::{Deserialize, Serialize};

/// How many messages' senders a session keeps.
pub const KEPT: usize = 256;

/// A session's record of who sent its input messages.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Senders {
    /// Message id and the person's user id, oldest first.
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    sent: VecDeque<(MessageId, String)>,
    /// The person who last wrote, which a platform message acts as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last: Option<String>,
}

impl Senders {
    pub fn is_empty(&self) -> bool {
        self.sent.is_empty() && self.last.is_none()
    }

    /// `user` sent `message`. A retry of the same message keeps one entry.
    pub fn record(&mut self, message: MessageId, user: String) {
        self.last = Some(user.clone());
        self.keep(message, user);
    }

    /// The platform admitted `message` on the session's behalf: it acts as
    /// the last person who wrote, when anyone has.
    pub fn inherit(&mut self, message: MessageId) {
        if let Some(user) = self.last.clone() {
            self.keep(message, user);
        }
    }

    /// Who sent `message`, when recorded.
    pub fn of(&self, message: MessageId) -> Option<&str> {
        self.sent
            .iter()
            .rev()
            .find(|(id, _)| *id == message)
            .map(|(_, user)| user.as_str())
    }

    fn keep(&mut self, message: MessageId, user: String) {
        if self.of(message).is_some() {
            return;
        }
        if self.sent.len() >= KEPT {
            self.sent.pop_front();
        }
        self.sent.push_back((message, user));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_acts_as_its_sender_and_a_platform_one_as_the_last() {
        let mut senders = Senders::default();
        let schedule_before_anyone = MessageId::new();
        senders.inherit(schedule_before_anyone);
        assert_eq!(senders.of(schedule_before_anyone), None);

        let mine = MessageId::new();
        senders.record(mine, "user-a".into());
        senders.record(mine, "user-b".into());
        assert_eq!(senders.of(mine), Some("user-a"), "a retry keeps the first");

        let wake = MessageId::new();
        senders.inherit(wake);
        assert_eq!(senders.of(wake), Some("user-b"));
        assert_eq!(senders.of(MessageId::new()), None, "unknown: no one");
    }

    #[test]
    fn only_the_latest_messages_are_kept() {
        let mut senders = Senders::default();
        let first = MessageId::new();
        senders.record(first, "user-a".into());
        for _ in 0..KEPT {
            senders.record(MessageId::new(), "user-a".into());
        }
        assert_eq!(senders.of(first), None);
        assert_eq!(senders.sent.len(), KEPT);
    }
}
