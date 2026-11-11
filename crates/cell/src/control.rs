//! What a cell keeps for its link to the control plane, as replicated state
//! (cp-dp-protocol.md): each org's config (directory cell only) and the
//! feed records not yet acknowledged.
//!
//! Decisions:
//! - Config applies in the directory cell's Raft, so the ordering rules
//!   (in order per org, a new epoch only as a full snapshot, an older epoch
//!   refused) hold across leader changes: a retried `PUT` lands on the
//!   applied version and is answered `AlreadyApplied`.
//! - Agent revisions are kept whole in the directory cell's state for now:
//!   they are small, and a session copies its revision's package when it
//!   is created, so a later revision or a removal never changes a session
//!   that started on the old one. They move to the object store when
//!   packages grow (step 7).
//! - Feed records are made at apply from committed events, numbered by the
//!   Raft index of their entry and their position in it, and stay in state
//!   until the control plane acknowledges them, so a new leader resends
//!   exactly what the old one had not delivered, under the same ids.

use std::collections::{BTreeMap, VecDeque};

use everruns_core::events::{Event, EventData};
use serde::{Deserialize, Serialize};

use crate::dp::{
    AgentDefinition, CommandAck, CommandStatus, ConfigAck, ConfigChange, ConfigDiff, ConfigKnown,
    ConfigUpdate, Connection, FeedRecord, OrgDelete, OrgEnable, OrgExport, OrgState,
    OrgStateCommand, Record, SealedSecret, TenantPolicy,
};
use crate::state::SessionState;

/// Every org's config, in the directory cell.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Orgs {
    pub orgs: BTreeMap<String, OrgConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OrgConfig {
    pub epoch: u64,
    pub applied_version: u64,
    /// The current revision of each agent.
    pub agents: BTreeMap<String, AgentDefinition>,
    /// The newest revision of each tenant secret, sealed.
    #[serde(default)]
    pub secrets: BTreeMap<String, SealedSecret>,
    #[serde(default)]
    pub connections: BTreeMap<String, Connection>,
    #[serde(default)]
    pub policy: TenantPolicy,
    /// Set by `OrgEnable`: the org's data key, wrapped with the data
    /// plane's key. Executors unwrap it to unseal the org's secrets.
    #[serde(default)]
    pub data_key: Option<WrappedKey>,
    #[serde(default)]
    pub state: Option<OrgState>,
    #[serde(default)]
    pub suspended_reason: Option<String>,
    /// Deleted: a tombstone; new sessions and config are refused.
    #[serde(default)]
    pub deleted: bool,
    /// Admin commands by id, newest last; the oldest are dropped past
    /// [`COMMANDS_KEPT`].
    #[serde(default)]
    pub commands: VecDeque<CommandStatus>,
}

/// An org's data key as `OrgEnable` delivered it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WrappedKey {
    pub data_key_id: String,
    /// Base64: nonce (12 bytes) then ciphertext.
    pub wrapped: String,
}

/// Admin command statuses kept per org, for `GET .../commands/{id}`.
pub const COMMANDS_KEPT: usize = 64;

impl OrgConfig {
    /// Suspended or deleted: no new turn starts for the org's sessions.
    pub fn stopped(&self) -> bool {
        self.deleted || self.state == Some(OrgState::Suspended)
    }

    pub fn command(&self, id: &str) -> Option<&CommandStatus> {
        self.commands.iter().find(|c| c.command_id == id)
    }

    fn record(&mut self, status: CommandStatus) {
        if let Some(slot) = self
            .commands
            .iter_mut()
            .find(|c| c.command_id == status.command_id)
        {
            *slot = status;
            return;
        }
        self.commands.push_back(status);
        while self.commands.len() > COMMANDS_KEPT {
            self.commands.pop_front();
        }
    }

    /// What this org holds that `known` (a restored control plane's view)
    /// does not: newer revisions count as unknown too.
    pub fn diff(&self, known: &ConfigKnown) -> ConfigDiff {
        let agents: BTreeMap<&str, u64> =
            known.agents.iter().map(|(id, r)| (id.as_str(), *r)).collect();
        let secrets: BTreeMap<&str, u64> =
            known.secrets.iter().map(|(id, r)| (id.as_str(), *r)).collect();
        ConfigDiff {
            agents: self
                .agents
                .values()
                .filter(|a| agents.get(a.agent_id.as_str()).is_none_or(|r| a.revision > *r))
                .map(|a| (a.agent_id.clone(), a.revision))
                .collect(),
            secrets: self
                .secrets
                .values()
                .filter(|s| secrets.get(s.secret_id.as_str()).is_none_or(|r| s.revision > *r))
                .map(|s| (s.secret_id.clone(), s.revision))
                .collect(),
            connections: self
                .connections
                .keys()
                .filter(|c| !known.connections.contains(c))
                .cloned()
                .collect(),
        }
    }
}

/// An admin command for one org, applied in the directory cell's Raft.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "admin")]
pub enum AdminCommand {
    Enable(OrgEnable),
    State(OrgStateCommand),
    Delete(OrgDelete),
    /// An export started; its status moves through [`AdminCommand::Progress`].
    Export(OrgExport),
    /// A long command's progress, from whoever runs it.
    Progress(CommandStatus),
}

impl AdminCommand {
    pub fn command_id(&self) -> &str {
        match self {
            AdminCommand::Enable(OrgEnable { command_id, .. })
            | AdminCommand::State(OrgStateCommand { command_id, .. })
            | AdminCommand::Delete(OrgDelete { command_id })
            | AdminCommand::Export(OrgExport { command_id, .. })
            | AdminCommand::Progress(CommandStatus { command_id, .. }) => command_id,
        }
    }
}

impl Orgs {
    /// Apply `update` to `org` by the protocol's ordering rules.
    pub fn apply(&mut self, org: &str, update: &ConfigUpdate) -> ConfigAck {
        let config = self.orgs.entry(org.to_owned()).or_default();
        let full = update.base_version == 0;
        if config.deleted {
            // A deleted org takes no config; the control plane stops its
            // outbox when it deletes the org.
            return ConfigAck::StaleEpoch {
                epoch: config.epoch.max(update.epoch),
            };
        }
        if update.epoch < config.epoch {
            return ConfigAck::StaleEpoch {
                epoch: config.epoch,
            };
        }
        let applied_version = config.applied_version;
        if update.epoch > config.epoch {
            // A restored control plane: only a full snapshot starts it.
            if !full {
                return ConfigAck::NeedFullSnapshot { applied_version };
            }
        } else if update.version <= applied_version {
            return ConfigAck::AlreadyApplied { applied_version };
        } else if !full && update.base_version != applied_version {
            return ConfigAck::NeedFullSnapshot { applied_version };
        }
        if full {
            // A full snapshot replaces the org's config. A secret the data
            // plane refreshed itself (higher revision) survives it.
            config.agents.clear();
            config.connections.clear();
            config.policy = TenantPolicy::default();
            let refreshed: Vec<SealedSecret> = update
                .changes
                .iter()
                .filter_map(ConfigChange::parse)
                .filter_map(|c| match c {
                    ConfigChange::Secret(s) => Some(s),
                    _ => None,
                })
                .filter_map(|s| {
                    config
                        .secrets
                        .get(&s.secret_id)
                        .filter(|held| held.revision > s.revision)
                        .cloned()
                })
                .collect();
            config.secrets.clear();
            for secret in refreshed {
                config.secrets.insert(secret.secret_id.clone(), secret);
            }
        }
        for change in update.changes.iter().filter_map(ConfigChange::parse) {
            match change {
                ConfigChange::Agent(agent) => {
                    let newer = config
                        .agents
                        .get(&agent.agent_id)
                        .is_none_or(|current| agent.revision > current.revision);
                    if newer {
                        config.agents.insert(agent.agent_id.clone(), agent);
                    }
                }
                ConfigChange::Secret(secret) => {
                    let newer = config
                        .secrets
                        .get(&secret.secret_id)
                        .is_none_or(|current| secret.revision > current.revision);
                    if newer {
                        config.secrets.insert(secret.secret_id.clone(), secret);
                    }
                }
                ConfigChange::Connection(connection) => {
                    config
                        .connections
                        .insert(connection.connection_id.clone(), connection);
                }
                ConfigChange::Policy(policy) => config.policy = policy,
                ConfigChange::Removal { of, id } => match of.as_str() {
                    "agent" => {
                        config.agents.remove(&id);
                    }
                    "secret" => {
                        config.secrets.remove(&id);
                    }
                    "connection" => {
                        config.connections.remove(&id);
                    }
                    _ => {}
                },
            }
        }
        config.epoch = update.epoch;
        config.applied_version = update.version;
        ConfigAck::Ok {
            applied_version: update.version,
        }
    }
}

impl Orgs {
    /// Apply an admin command to `org`. Idempotent by `command_id`: a
    /// command already recorded is acknowledged again without effect.
    pub fn admin(&mut self, org: &str, command: AdminCommand) -> CommandAck {
        let config = self.orgs.entry(org.to_owned()).or_default();
        let done = |id: &str| CommandStatus {
            command_id: id.to_owned(),
            state: "done".into(),
            progress: Some(1.0),
            error: None,
        };
        let ack = |id: &str| CommandAck {
            command_id: id.to_owned(),
            accepted: true,
            error: None,
        };
        let id = command.command_id().to_owned();
        if let AdminCommand::Progress(status) = command {
            // Finished commands don't move again.
            if config.command(&id).is_none_or(|c| c.state == "running") {
                config.record(status);
            }
            return ack(&id);
        }
        if config.command(&id).is_some() {
            return ack(&id);
        }
        if config.deleted {
            return CommandAck {
                command_id: id,
                accepted: false,
                error: Some("the org is deleted".into()),
            };
        }
        match command {
            AdminCommand::Enable(enable) => {
                config.data_key = Some(WrappedKey {
                    data_key_id: enable.data_key_id,
                    wrapped: enable.wrapped_data_key,
                });
                config.policy = enable.policy;
                config.state.get_or_insert(OrgState::Active);
                config.record(done(&id));
            }
            AdminCommand::State(state) => {
                config.state = Some(state.state);
                config.suspended_reason = match state.state {
                    OrgState::Suspended => state.reason,
                    OrgState::Active => None,
                };
                config.record(done(&id));
            }
            AdminCommand::Delete(_) => {
                config.deleted = true;
                config.agents.clear();
                config.secrets.clear();
                config.connections.clear();
                config.data_key = None;
                config.record(done(&id));
            }
            AdminCommand::Export(_) => config.record(CommandStatus {
                command_id: id.clone(),
                state: "running".into(),
                progress: Some(0.0),
                error: None,
            }),
            AdminCommand::Progress(_) => {}
        }
        ack(&id)
    }

    /// The orgs whose sessions may not start new turns, and the reason.
    pub fn stopped(&self) -> BTreeMap<String, String> {
        self.orgs
            .iter()
            .filter(|(_, c)| c.stopped())
            .map(|(org, c)| {
                let why = if c.deleted {
                    "deleted".to_owned()
                } else {
                    c.suspended_reason.clone().unwrap_or_else(|| "suspended".into())
                };
                (org.clone(), why)
            })
            .collect()
    }
}

/// Feed records waiting for the control plane's acknowledgement.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Feed {
    pub records: VecDeque<FeedRecord>,
    /// `(index, ordinal)` of the last record acknowledged.
    #[serde(default)]
    pub acked: (u64, u32),
}

impl Feed {
    pub fn push(&mut self, index: u64, org: &str, at_unix_ms: i64, record: Record) {
        let ordinal = match self.records.back() {
            Some(last) if last.index == index => last.ordinal + 1,
            _ => 0,
        };
        self.records.push_back(FeedRecord {
            index,
            ordinal,
            org_id: org.to_owned(),
            at_unix_ms,
            record,
        });
    }

    /// Drop the records the control plane stored, through `through`.
    pub fn ack(&mut self, through: (u64, u32)) {
        while self
            .records
            .front()
            .is_some_and(|r| (r.index, r.ordinal) <= through)
        {
            self.records.pop_front();
        }
        self.acked = self.acked.max(through);
    }

    /// The records `session`'s committed `event` makes for its tenant.
    pub fn observe(&mut self, index: u64, session: &str, s: &SessionState, event: &Event) {
        let Some(org) = s.tenant.as_deref() else {
            return;
        };
        let EventData::LlmGeneration(data) = &event.data else {
            return;
        };
        let at = event.ts.timestamp_millis();
        let usage = data.metadata.usage.as_ref();
        let cost = usage
            .and_then(|u| u.effective_cost_usd())
            .map(|usd| (usd * 1_000_000.0).ceil() as i64)
            .unwrap_or(0);
        let record = Record::Usage {
            session_id: session.to_owned(),
            turn_id: event
                .context
                .turn_id
                .map(|t| t.to_string())
                .unwrap_or_default(),
            // The event's id is stable: a replay of the same entry
            // makes the same id, so usage is billed once.
            attempt_id: event.id.to_string(),
            unresolved: false,
            agent_id: s.agent.clone(),
            provider: data.metadata.provider.clone().unwrap_or_default(),
            model: data.metadata.model.clone(),
            input_tokens: usage.map_or(0, |u| u64::from(u.input_tokens)),
            output_tokens: usage.map_or(0, |u| u64::from(u.output_tokens)),
            cached_tokens: usage.and_then(|u| u.cache_read_tokens).map_or(0, u64::from),
            cost_micros: cost,
            kind: "model".into(),
            agent_hash: String::new(),
        };
        self.push(index, org, at, record);
    }

    /// `session` recorded a turn's outcome: its summary for the tenant.
    /// Decision: the summary follows the outcome record, not the terminal
    /// event, because the turn count moves with the outcome.
    pub fn turn_finished(&mut self, index: u64, session: &str, s: &SessionState) {
        let Some(org) = s.tenant.as_deref() else {
            return;
        };
        let last = s.events.iter().rev().find(|e| {
            matches!(
                e.data,
                EventData::TurnCompleted(_)
                    | EventData::TurnFailed(_)
                    | EventData::TurnCancelled(_)
            )
        });
        let status = match last.map(|e| &e.data) {
            Some(EventData::TurnFailed(_)) => "failed",
            _ => "idle",
        };
        let at = last.map_or(s.created_ms, |e| e.ts.timestamp_millis());
        let record = Record::Session {
            session_id: session.to_owned(),
            session_version: s.last_sequence().unwrap_or(0).max(0) as u64,
            agent_id: s.agent.clone(),
            status: status.into(),
            created_unix_ms: s.created_ms,
            last_activity_unix_ms: at,
            turns: s.turns_completed(),
            title: String::new(),
            cost_micros: 0,
        };
        self.push(index, org, at, record);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use serde_json::json;

    fn agent(id: &str, revision: u64) -> serde_json::Value {
        json!({ "kind": "Agent", "agent_id": id, "revision": revision,
                "package": { "name": id, "instructions": format!("r{revision}") } })
    }

    fn update(
        epoch: u64,
        version: u64,
        base: u64,
        changes: Vec<serde_json::Value>,
    ) -> ConfigUpdate {
        ConfigUpdate {
            epoch,
            version,
            base_version: base,
            changes,
        }
    }

    #[test]
    fn config_applies_in_order_and_once() {
        let mut orgs = Orgs::default();
        let ok = |v| ConfigAck::Ok { applied_version: v };
        assert_eq!(
            orgs.apply("o", &update(1, 1, 0, vec![agent("a", 1)])),
            ok(1)
        );
        assert_eq!(
            orgs.apply("o", &update(1, 2, 1, vec![agent("a", 2)])),
            ok(2)
        );
        assert_eq!(
            orgs.apply("o", &update(1, 2, 1, vec![agent("a", 2)])),
            ConfigAck::AlreadyApplied { applied_version: 2 },
            "a retried PUT"
        );
        assert_eq!(
            orgs.apply("o", &update(1, 4, 3, vec![])),
            ConfigAck::NeedFullSnapshot { applied_version: 2 },
            "a gap"
        );
        assert_eq!(orgs.orgs["o"].agents["a"].revision, 2);
        // Revisions only move forward, whatever order they arrive in.
        assert_eq!(
            orgs.apply("o", &update(1, 3, 2, vec![agent("a", 1)])),
            ok(3)
        );
        assert_eq!(orgs.orgs["o"].agents["a"].revision, 2);
    }

    #[test]
    fn a_restored_control_plane_starts_with_a_full_snapshot() {
        let mut orgs = Orgs::default();
        orgs.apply("o", &update(1, 5, 0, vec![agent("a", 3), agent("b", 1)]));
        assert_eq!(
            orgs.apply("o", &update(2, 1, 1, vec![])),
            ConfigAck::NeedFullSnapshot { applied_version: 5 },
            "a new epoch's first update must be a full snapshot"
        );
        assert_eq!(
            orgs.apply("o", &update(2, 1, 0, vec![agent("a", 3)])),
            ConfigAck::Ok { applied_version: 1 }
        );
        assert!(
            !orgs.orgs["o"].agents.contains_key("b"),
            "the snapshot is whole"
        );
        assert_eq!(
            orgs.apply("o", &update(1, 6, 5, vec![agent("b", 2)])),
            ConfigAck::StaleEpoch { epoch: 2 },
            "the old control plane is refused"
        );
        assert!(!orgs.orgs["o"].agents.contains_key("b"));
    }

    fn secret(id: &str, revision: u64) -> serde_json::Value {
        json!({ "kind": "Secret", "secret_id": id, "revision": revision,
                "ciphertext": format!("c{revision}"), "nonce": "n", "data_key_id": "k" })
    }

    #[test]
    fn secrets_connections_and_policy_follow_the_same_rules() {
        let mut orgs = Orgs::default();
        let connection = json!({
            "kind": "Connection", "connection_id": "gh", "connection_kind": "github",
            "token_secret_id": "t", "refresh_owner": "DataPlane"
        });
        let policy = json!({ "kind": "Policy", "features": ["stage1"] });
        orgs.apply(
            "o",
            &update(1, 1, 0, vec![secret("t", 2), connection, policy]),
        );
        let config = &orgs.orgs["o"];
        assert_eq!(config.secrets["t"].revision, 2);
        assert_eq!(config.connections["gh"].kind, "github");
        assert_eq!(config.policy.features, vec!["stage1".to_owned()]);

        // An older revision (a delayed or replayed change) never wins.
        orgs.apply("o", &update(1, 2, 1, vec![secret("t", 1)]));
        assert_eq!(orgs.orgs["o"].secrets["t"].ciphertext, "c2");

        // A full snapshot replaces config, but a secret the data plane
        // refreshed itself (newer than the snapshot's) is kept.
        orgs.apply("o", &update(1, 3, 0, vec![secret("t", 1), agent("a", 1)]));
        let config = &orgs.orgs["o"];
        assert_eq!(config.secrets["t"].revision, 2);
        assert!(config.connections.is_empty());
        assert!(config.policy.features.is_empty());

        orgs.apply(
            "o",
            &update(1, 4, 3, vec![json!({ "kind": "Removal", "of": "secret", "id": "t" })]),
        );
        assert!(orgs.orgs["o"].secrets.is_empty());
    }

    fn state(id: &str, state: OrgState) -> AdminCommand {
        AdminCommand::State(OrgStateCommand {
            command_id: id.into(),
            state,
            reason: Some("billing".into()),
        })
    }

    #[test]
    fn admin_commands_apply_once_and_a_deleted_org_takes_nothing() {
        let mut orgs = Orgs::default();
        let enable = AdminCommand::Enable(OrgEnable {
            command_id: "c1".into(),
            wrapped_data_key: "w".into(),
            data_key_id: "k1".into(),
            policy: TenantPolicy::default(),
        });
        assert!(orgs.admin("o", enable.clone()).accepted);
        assert_eq!(orgs.orgs["o"].state, Some(OrgState::Active));

        assert!(orgs.admin("o", state("c2", OrgState::Suspended)).accepted);
        assert_eq!(orgs.stopped()["o"], "billing");
        assert!(orgs.admin("o", state("c3", OrgState::Active)).accepted);
        assert!(orgs.stopped().is_empty());
        // A retried command changes nothing: the org stays active.
        assert!(orgs.admin("o", state("c2", OrgState::Suspended)).accepted);
        assert!(orgs.stopped().is_empty());

        assert!(
            orgs.admin("o", AdminCommand::Delete(OrgDelete { command_id: "c4".into() }))
                .accepted
        );
        assert_eq!(orgs.stopped()["o"], "deleted");
        assert!(orgs.orgs["o"].data_key.is_none());
        let refused = orgs.admin("o", state("c5", OrgState::Active));
        assert!(!refused.accepted);
        assert!(matches!(
            orgs.apply("o", &update(1, 1, 0, vec![agent("a", 1)])),
            ConfigAck::StaleEpoch { .. }
        ));
        assert!(orgs.orgs["o"].agents.is_empty());
    }

    #[test]
    fn an_export_runs_until_its_progress_finishes_it() {
        let mut orgs = Orgs::default();
        let export = AdminCommand::Export(OrgExport {
            command_id: "e".into(),
            export_id: "x".into(),
        });
        orgs.admin("o", export);
        assert_eq!(orgs.orgs["o"].command("e").unwrap().state, "running");
        let status = |state: &str| {
            AdminCommand::Progress(CommandStatus {
                command_id: "e".into(),
                state: state.into(),
                progress: None,
                error: None,
            })
        };
        orgs.admin("o", status("done"));
        orgs.admin("o", status("failed"));
        assert_eq!(
            orgs.orgs["o"].command("e").unwrap().state,
            "done",
            "a finished command does not move again"
        );
    }

    #[test]
    fn the_diff_names_what_a_restored_control_plane_lost() {
        let mut orgs = Orgs::default();
        orgs.apply(
            "o",
            &update(1, 1, 0, vec![agent("a", 3), agent("b", 1), secret("s", 2)]),
        );
        let known = ConfigKnown {
            agents: vec![("a".into(), 2), ("b".into(), 1)],
            secrets: vec![],
            connections: vec![],
        };
        let diff = orgs.orgs["o"].diff(&known);
        assert_eq!(diff.agents, vec![("a".to_owned(), 3)]);
        assert_eq!(diff.secrets, vec![("s".to_owned(), 2)]);
    }

    #[test]
    fn feed_records_number_by_entry_and_leave_once_acknowledged() {
        let mut feed = Feed::default();
        for (index, n) in [(7, 2), (9, 1)] {
            for _ in 0..n {
                feed.push(index, "o", 0, Record::ConfigApplied { version: 1 });
            }
        }
        let ids: Vec<(u64, u32)> = feed.records.iter().map(|r| (r.index, r.ordinal)).collect();
        assert_eq!(ids, vec![(7, 0), (7, 1), (9, 0)]);
        feed.ack((7, 1));
        assert_eq!(feed.records.len(), 1);
        feed.ack((7, 0));
        assert_eq!(feed.records.len(), 1, "an older ack changes nothing");
        assert_eq!(feed.acked, (7, 1));
    }
}
