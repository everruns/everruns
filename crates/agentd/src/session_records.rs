//! What a session's tools keep about the session on its shard: its
//! resources (fleet sandboxes, activated skills), the leases of what its
//! tools started at a provider, and dreaming's maintenance state.
//!
//! Decisions:
//! - These are the session's own state, so they live in its replicated
//!   platform state and move, snapshot and restore with it. A shard asks no
//!   control plane for them; a standalone agentd keeps them the same way.
//! - Every write is a [`Command::Platform`] on the session's shard, applied
//!   by the state machine with the time the proposer chose, so every
//!   replica applies the same change and a retry finds what it made.
//! - A resource or a lease that changes goes on the feed
//!   (`Record::SessionResource`, `Record::Lease`): the control plane
//!   mirrors them into its own tables, where the console lists the
//!   resources and the lease sweeper stops a sandbox a session left
//!   running. The feed reports; nothing waits on the control plane.
//! - A lease is keyed by what it leases (provider, type, external id), so
//!   renewing one extends it instead of adding another, as the control
//!   plane's store does.
//! - Dreaming's state is the session's: its turn counter, its dream job and
//!   the daily budgets of the Memory scopes it dreams into. Budgets are
//!   per session here, as on Engine's own hosting (`SessionDreaming`); the
//!   executor's sweep finds a due dream from the sessions it mirrors
//!   ([`Platform::dream_due`]).

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use everruns_contracts::dataplane::Record;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::runtime::session_resource::{
    RegisterSessionResource, SessionResourceEntry, SessionResourceFilter, SessionResourceStatus,
};
use everruns_contracts::runtime::session_services::{LeasedResourceStore, SessionResourceRegistry};
use everruns_contracts::runtime::{LeasedResource, LeasedResourceStatus, UpsertLeasedResource};
use everruns_contracts::typed_id::{EventId, LeasedResourceId, SessionId};
use everruns_core::dreaming::{DreamStateRow, DreamingStore};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::hosting::Port;
use crate::platform::{Op, Platform};
use crate::state::{Change, Command, Response, SessionState};

/// Longest dreaming state key and value, as the control plane allowed.
const MAX_DREAM_KEY: usize = 512;
const MAX_DREAM_VALUE: usize = 1024 * 1024;
/// Resources and leases one session may hold.
const MAX_RESOURCES: usize = 256;

/// A change to a session's records.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecordOp {
    RegisterResource {
        entry: RegisterSessionResource,
    },
    ResourceStatus {
        resource_id: String,
        status: SessionResourceStatus,
    },
    DeregisterResource {
        resource_id: String,
    },
    /// `id` is the lease's id if it is new; a renewal keeps its own.
    UpsertLease {
        input: UpsertLeasedResource,
        id: LeasedResourceId,
    },
    ReleaseLease {
        provider: String,
        resource_type: String,
        external_id: String,
    },
    /// The lease sweep takes a due lease (`lease_sweep.rs`): expired and
    /// active or failed before, or claimed longer than `stale_after_seconds`
    /// ago by a sweep that did not finish. Answers the claimed lease, or
    /// null when it was not due.
    ClaimLease {
        provider: String,
        resource_type: String,
        external_id: String,
        stale_after_seconds: u32,
    },
    /// The sweep's outcome for the claim made at `claimed_at`: released, or
    /// failed with `error` and due again in `retry_after_seconds`. A claim
    /// that is no longer the lease's (renewed, reclaimed) changes nothing.
    LeaseCleaned {
        provider: String,
        resource_type: String,
        external_id: String,
        claimed_at: DateTime<Utc>,
        error: Option<String>,
        retry_after_seconds: u32,
    },
    /// Dreaming's compare-exchange: create `key` when `expected` is none
    /// and it is absent, else replace it at revision `expected`.
    Dream {
        key: String,
        expected: Option<i64>,
        value: String,
        /// Whether the row is the session's own (its job), which the sweep
        /// finds by `due_at`.
        #[serde(default)]
        owned: bool,
        due_at: Option<i64>,
    },
}

/// The records part of a session's platform state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Records {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resources: BTreeMap<String, SessionResourceEntry>,
    /// By [`lease_key`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub leases: BTreeMap<String, LeasedResource>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dreaming: BTreeMap<String, DreamStateRow>,
}

impl Records {
    pub fn is_empty(&self) -> bool {
        self.resources.is_empty() && self.leases.is_empty() && self.dreaming.is_empty()
    }

    /// The leases due for cleanup at `now` (see [`RecordOp::ClaimLease`]).
    pub fn due_leases(
        &self,
        now: DateTime<Utc>,
        stale_after_seconds: u32,
    ) -> impl Iterator<Item = &LeasedResource> {
        self.leases
            .values()
            .filter(move |lease| lease_due(lease, now, stale_after_seconds))
    }
}

fn lease_due(lease: &LeasedResource, now: DateTime<Utc>, stale_after_seconds: u32) -> bool {
    match lease.status {
        LeasedResourceStatus::Active | LeasedResourceStatus::CleanupFailed => {
            lease.lease_expires_at <= now
        }
        LeasedResourceStatus::Cleaning => lease.cleanup_started_at.is_some_and(|started| {
            started <= now - chrono::Duration::seconds(i64::from(stale_after_seconds))
        }),
        LeasedResourceStatus::Released => false,
    }
}

/// What a lease leases.
fn lease_key(provider: &str, resource_type: &str, external_id: &str) -> String {
    format!("{provider}\u{1f}{resource_type}\u{1f}{external_id}")
}

fn at(now_ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(now_ms).unwrap_or_default()
}

fn rejected(reason: impl Into<String>) -> Response {
    Response::Rejected {
        reason: reason.into(),
    }
}

impl Platform {
    /// Whether `session`'s dream is due at `now` (unix seconds).
    pub fn dream_due(&self, session: SessionId, now: i64) -> bool {
        self.records
            .dreaming
            .get(&everruns_core::engine::dreaming::runner::job_key(session))
            .and_then(|row| row.due_at)
            .is_some_and(|due| due <= now)
    }
}

/// Apply `op` to `state`; a resource or lease that changed is put in
/// `notices` for the feed.
pub(crate) fn apply(
    state: &mut SessionState,
    session: SessionId,
    op: RecordOp,
    now_ms: i64,
    changes: &mut Vec<Change>,
    notices: &mut Vec<Record>,
) -> Response {
    let now = at(now_ms);
    let records = &mut state.platform.records;
    let response = match op {
        RecordOp::RegisterResource { entry } => {
            if entry.session_id != session {
                return rejected("a resource of another session");
            }
            if !records.resources.contains_key(&entry.resource_id)
                && records.resources.len() >= MAX_RESOURCES
            {
                return rejected(format!("at most {MAX_RESOURCES} resources per session"));
            }
            let created_at = records
                .resources
                .get(&entry.resource_id)
                .map_or(now, |existing| existing.created_at);
            let stored = SessionResourceEntry {
                resource_id: entry.resource_id.clone(),
                session_id: session,
                kind: entry.kind,
                display_name: entry.display_name,
                status: entry.status,
                metadata: entry.metadata,
                created_at,
                updated_at: now,
            };
            records
                .resources
                .insert(entry.resource_id.clone(), stored.clone());
            notices.push(resource_notice(session, &entry.resource_id, Some(&stored)));
            json!(stored)
        }
        RecordOp::ResourceStatus {
            resource_id,
            status,
        } => match records.resources.get_mut(&resource_id) {
            Some(entry) => {
                entry.status = status;
                entry.updated_at = now;
                let entry = entry.clone();
                notices.push(resource_notice(session, &resource_id, Some(&entry)));
                json!(entry)
            }
            None => Value::Null,
        },
        RecordOp::DeregisterResource { resource_id } => {
            let removed = records.resources.remove(&resource_id).is_some();
            if removed {
                notices.push(resource_notice(session, &resource_id, None));
            }
            json!(removed)
        }
        RecordOp::UpsertLease { input, id } => {
            if input.session_id != session {
                return rejected("a lease of another session");
            }
            let key = lease_key(&input.provider, &input.resource_type, &input.external_id);
            if !records.leases.contains_key(&key) && records.leases.len() >= MAX_RESOURCES {
                return rejected(format!("at most {MAX_RESOURCES} leases per session"));
            }
            let expires = now + chrono::Duration::seconds(i64::from(input.lease_duration_seconds));
            let lease = match records.leases.remove(&key) {
                Some(mut lease) => {
                    lease.display_name = input.display_name.or(lease.display_name);
                    lease.owner_user_id = input.owner_user_id.or(lease.owner_user_id);
                    lease.connection_id = input.connection_id.or(lease.connection_id);
                    lease.lease_duration_seconds = input.lease_duration_seconds;
                    lease.last_touched_at = now;
                    lease.lease_expires_at = expires;
                    lease.status = LeasedResourceStatus::Active;
                    if !input.metadata.is_null() {
                        lease.metadata = input.metadata;
                    }
                    lease.updated_at = now;
                    lease
                }
                None => LeasedResource {
                    id,
                    session_id: Some(session),
                    provider: input.provider,
                    resource_type: input.resource_type,
                    external_id: input.external_id,
                    display_name: input.display_name,
                    status: LeasedResourceStatus::Active,
                    owner_user_id: input.owner_user_id,
                    connection_id: input.connection_id,
                    lease_duration_seconds: input.lease_duration_seconds,
                    last_touched_at: now,
                    lease_expires_at: expires,
                    cleanup_started_at: None,
                    cleanup_completed_at: None,
                    cleanup_attempts: 0,
                    last_cleanup_error: None,
                    metadata: input.metadata,
                    created_at: now,
                    updated_at: now,
                },
            };
            records.leases.insert(key, lease.clone());
            notices.push(Record::Lease {
                session_id: session.to_string(),
                lease: json!(lease),
            });
            // Registered as one of the session's resources too, as the
            // control plane's store does, so the agent and the console see
            // it and ownership checks find it.
            let id = lease.id.to_string();
            let entry = SessionResourceEntry {
                resource_id: id.clone(),
                session_id: session,
                kind: lease.resource_type.clone(),
                display_name: lease
                    .display_name
                    .clone()
                    .unwrap_or_else(|| lease.external_id.clone()),
                status: SessionResourceStatus::Active,
                metadata: everruns_contracts::runtime::resource_ownership::lease_registry_metadata(
                    lease.metadata.clone(),
                    &id,
                    &lease.provider,
                    &lease.resource_type,
                    &lease.external_id,
                ),
                created_at: records
                    .resources
                    .get(&id)
                    .map_or(now, |existing| existing.created_at),
                updated_at: now,
            };
            records.resources.insert(id.clone(), entry.clone());
            notices.push(resource_notice(session, &id, Some(&entry)));
            json!(lease)
        }
        RecordOp::ReleaseLease {
            provider,
            resource_type,
            external_id,
        } => {
            let key = lease_key(&provider, &resource_type, &external_id);
            release(records, &key, session, now, notices)
        }
        RecordOp::ClaimLease {
            provider,
            resource_type,
            external_id,
            stale_after_seconds,
        } => {
            let key = lease_key(&provider, &resource_type, &external_id);
            match records.leases.get_mut(&key) {
                Some(lease) if lease_due(lease, now, stale_after_seconds) => {
                    lease.status = LeasedResourceStatus::Cleaning;
                    lease.cleanup_started_at = Some(now);
                    lease.cleanup_attempts += 1;
                    lease.last_cleanup_error = None;
                    lease.updated_at = now;
                    let lease = lease.clone();
                    notices.push(Record::Lease {
                        session_id: session.to_string(),
                        lease: json!(lease),
                    });
                    json!(lease)
                }
                _ => Value::Null,
            }
        }
        RecordOp::LeaseCleaned {
            provider,
            resource_type,
            external_id,
            claimed_at,
            error,
            retry_after_seconds,
        } => {
            let key = lease_key(&provider, &resource_type, &external_id);
            let ours = records.leases.get(&key).is_some_and(|lease| {
                lease.status == LeasedResourceStatus::Cleaning
                    && lease.cleanup_started_at == Some(claimed_at)
            });
            match (ours, error) {
                (false, _) => Value::Null,
                (true, None) => release(records, &key, session, now, notices),
                (true, Some(error)) => {
                    let Some(lease) = records.leases.get_mut(&key) else {
                        return rejected("lease vanished");
                    };
                    lease.status = LeasedResourceStatus::CleanupFailed;
                    lease.cleanup_started_at = None;
                    lease.last_cleanup_error = Some(error);
                    lease.lease_expires_at =
                        now + chrono::Duration::seconds(i64::from(retry_after_seconds));
                    lease.updated_at = now;
                    let lease = lease.clone();
                    notices.push(Record::Lease {
                        session_id: session.to_string(),
                        lease: json!(lease),
                    });
                    json!(lease)
                }
            }
        }
        RecordOp::Dream {
            key,
            expected,
            value,
            owned,
            due_at,
        } => {
            if key.len() > MAX_DREAM_KEY || value.len() > MAX_DREAM_VALUE {
                return rejected("dreaming state exceeds limit");
            }
            let current = records.dreaming.get(&key).map(|row| row.revision);
            if current != expected {
                return Response::Value {
                    value: json!(false),
                };
            }
            records.dreaming.insert(
                key,
                DreamStateRow {
                    revision: expected.map_or(1, |revision| revision + 1),
                    value,
                    session_id: owned.then_some(session),
                    due_at,
                },
            );
            json!(true)
        }
    };
    changes.push(Change::Session(session));
    Response::Value { value: response }
}

/// Mark the lease at `key` released, and its resource entry with it.
fn release(
    records: &mut Records,
    key: &str,
    session: SessionId,
    now: DateTime<Utc>,
    notices: &mut Vec<Record>,
) -> Value {
    match records.leases.get_mut(key) {
        Some(lease) => {
            lease.status = LeasedResourceStatus::Released;
            lease.cleanup_completed_at = Some(now);
            lease.updated_at = now;
            let lease = lease.clone();
            notices.push(Record::Lease {
                session_id: session.to_string(),
                lease: json!(lease),
            });
            let id = lease.id.to_string();
            if let Some(entry) = records.resources.get_mut(&id) {
                entry.status = SessionResourceStatus::Released;
                entry.updated_at = now;
                let entry = entry.clone();
                notices.push(resource_notice(session, &id, Some(&entry)));
            }
            json!(lease)
        }
        None => Value::Null,
    }
}

fn resource_notice(session: SessionId, id: &str, entry: Option<&SessionResourceEntry>) -> Record {
    Record::SessionResource {
        session_id: session.to_string(),
        resource_id: id.to_owned(),
        resource: entry.map(|entry| json!(entry)).unwrap_or(Value::Null),
    }
}

/// The session's records through its shard.
#[derive(Clone)]
pub(crate) struct ShardRecords {
    port: Port,
    session: SessionId,
}

impl ShardRecords {
    pub(crate) fn new(port: Port, session: SessionId) -> Self {
        Self { port, session }
    }

    async fn propose(&self, op: RecordOp) -> Result<Value> {
        self.port
            .propose(
                self.session,
                Command::Platform {
                    session: self.session,
                    action: Op::Records(op),
                    now_ms: crate::node::now_ms(),
                    seed: EventId::new(),
                },
            )
            .await
    }

    fn own(&self, session: SessionId) -> Result<()> {
        if session == self.session {
            Ok(())
        } else {
            Err(AgentLoopError::store("another session's records"))
        }
    }

    async fn read<T>(&self, f: impl FnOnce(&Records) -> T) -> Result<T> {
        self.port
            .read(self.session, |s| f(&s.platform.records))
            .await
    }

    /// Claim `lease` for the lease sweep; the claimed lease, or none when
    /// it was not due any more.
    pub(crate) async fn claim_lease(
        &self,
        lease: &LeasedResource,
        stale_after_seconds: u32,
    ) -> Result<Option<LeasedResource>> {
        parsed(
            self.propose(RecordOp::ClaimLease {
                provider: lease.provider.clone(),
                resource_type: lease.resource_type.clone(),
                external_id: lease.external_id.clone(),
                stale_after_seconds,
            })
            .await?,
        )
    }

    /// Record the sweep's outcome for the claim `claimed` holds.
    pub(crate) async fn lease_cleaned(
        &self,
        claimed: &LeasedResource,
        error: Option<String>,
        retry_after_seconds: u32,
    ) -> Result<Option<LeasedResource>> {
        let Some(claimed_at) = claimed.cleanup_started_at else {
            return Ok(None);
        };
        parsed(
            self.propose(RecordOp::LeaseCleaned {
                provider: claimed.provider.clone(),
                resource_type: claimed.resource_type.clone(),
                external_id: claimed.external_id.clone(),
                claimed_at,
                error,
                retry_after_seconds,
            })
            .await?,
        )
    }
}

fn parsed<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|e| AgentLoopError::store(e.to_string()))
}

#[async_trait]
impl SessionResourceRegistry for ShardRecords {
    async fn register(&self, entry: RegisterSessionResource) -> Result<SessionResourceEntry> {
        self.own(entry.session_id)?;
        parsed(self.propose(RecordOp::RegisterResource { entry }).await?)
    }

    async fn update_status(
        &self,
        session_id: SessionId,
        resource_id: &str,
        status: SessionResourceStatus,
    ) -> Result<Option<SessionResourceEntry>> {
        self.own(session_id)?;
        parsed(
            self.propose(RecordOp::ResourceStatus {
                resource_id: resource_id.to_owned(),
                status,
            })
            .await?,
        )
    }

    async fn get(
        &self,
        session_id: SessionId,
        resource_id: &str,
    ) -> Result<Option<SessionResourceEntry>> {
        self.own(session_id)?;
        self.read(|records| records.resources.get(resource_id).cloned())
            .await
    }

    async fn list(
        &self,
        session_id: SessionId,
        filter: Option<&SessionResourceFilter>,
    ) -> Result<Vec<SessionResourceEntry>> {
        self.own(session_id)?;
        let (kind, status) = filter.map_or((None, None), |f| (f.kind.clone(), f.status));
        self.read(|records| {
            records
                .resources
                .values()
                .filter(|entry| kind.as_ref().is_none_or(|kind| &entry.kind == kind))
                .filter(|entry| status.is_none_or(|status| entry.status == status))
                .cloned()
                .collect()
        })
        .await
    }

    async fn deregister(&self, session_id: SessionId, resource_id: &str) -> Result<bool> {
        self.own(session_id)?;
        parsed(
            self.propose(RecordOp::DeregisterResource {
                resource_id: resource_id.to_owned(),
            })
            .await?,
        )
    }
}

#[async_trait]
impl LeasedResourceStore for ShardRecords {
    async fn upsert_resource(&self, input: UpsertLeasedResource) -> Result<LeasedResource> {
        self.own(input.session_id)?;
        parsed(
            self.propose(RecordOp::UpsertLease {
                input,
                id: LeasedResourceId::new(),
            })
            .await?,
        )
    }

    async fn release_resource(
        &self,
        session_id: SessionId,
        provider: &str,
        resource_type: &str,
        external_id: &str,
    ) -> Result<Option<LeasedResource>> {
        self.own(session_id)?;
        parsed(
            self.propose(RecordOp::ReleaseLease {
                provider: provider.to_owned(),
                resource_type: resource_type.to_owned(),
                external_id: external_id.to_owned(),
            })
            .await?,
        )
    }

    async fn list_resources(&self, session_id: SessionId) -> Result<Vec<LeasedResource>> {
        self.own(session_id)?;
        self.read(|records| records.leases.values().cloned().collect())
            .await
    }
}

/// Dreaming's state of one session (`DreamingStore`), scoped to it as an
/// Engine session's store is: it commits no other session's job and
/// discovers none; discovery is the executor's sweep.
#[async_trait]
impl DreamingStore for ShardRecords {
    async fn get(&self, _org: i64, key: &str) -> Result<Option<DreamStateRow>> {
        self.read(|records| records.dreaming.get(key).cloned())
            .await
    }

    async fn compare_exchange(
        &self,
        _org: i64,
        key: &str,
        expected: Option<i64>,
        value: &str,
        session_id: Option<SessionId>,
        due_at: Option<i64>,
    ) -> Result<bool> {
        if session_id.is_some_and(|session| session != self.session) {
            return Err(AgentLoopError::config("cross-session maintenance access"));
        }
        parsed(
            self.propose(RecordOp::Dream {
                key: key.to_owned(),
                expected,
                value: value.to_owned(),
                owned: session_id.is_some(),
                due_at,
            })
            .await?,
        )
    }

    async fn due(&self, _now: i64, _limit: u32) -> Result<Vec<(i64, SessionId)>> {
        Ok(Vec::new())
    }
}

/// Give an activation's runtime the session's records.
pub(crate) fn attach(
    mut backends: everruns_core::host::HostBackends,
    records: ShardRecords,
) -> everruns_core::host::HostBackends {
    let records = Arc::new(records);
    backends.extensions.stores.session_resources = Some(records.clone());
    backends.extensions.stores.leased_resources = Some(records);
    backends
}

#[cfg(test)]
#[path = "session_records_tests.rs"]
mod tests;
