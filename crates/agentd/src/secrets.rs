//! Secrets the data plane refreshes itself (design.md, "Credentials and
//! suspension have a freshness bound"; step 11.2): an org's MCP OAuth
//! tokens. The control plane seals a token set with the org's data key and
//! sends it as a secret with a `RefreshOwner::DataPlane` connection; an
//! executor that finds the token near its expiry renews it at the token
//! endpoint and writes the new set back here, sealed again, as the next
//! revision. The directory cell keeps it, every executor reads it from
//! there, and the feed carries it to the control plane (`SecretRefreshed`).
//!
//! Decisions:
//! - One refresher at a time per secret: an executor first leases the
//!   secret for the revision it read. A provider that rotates refresh
//!   tokens invalidates the old one at the first refresh, so two executors
//!   refreshing the same revision would leave one of them, and possibly
//!   the grant, broken. An executor refused the lease waits for the
//!   holder's write and reads the new revision.
//! - A lease lasts [`LEASE_MS`] at most: a refresher that died holds the
//!   secret that long, never forever.
//! - Only the lease holder writes, and only the next revision. A control
//!   plane update carrying an older revision never replaces it (the
//!   revision rule in `Orgs::apply`), so a slow full snapshot cannot bring
//!   back a token whose refresh token was already spent.
//! - The lease lives in replicated state, with the proposer's clock: every
//!   node of the directory cell agrees who holds it.

use serde::{Deserialize, Serialize};

use crate::control::{Feed, Orgs};
use crate::dp::{Record, SealedSecret};
use crate::state::Response;

/// How long a refresh lease holds at most.
pub const LEASE_MS: i64 = 30_000;

/// Who refreshes a secret now, and until when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub holder: String,
    pub until_ms: i64,
}

/// What an executor asks of the directory cell for one of an org's secrets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SecretOp {
    /// Lease `secret_id` to refresh it from `revision`, the one read.
    Lease {
        secret_id: String,
        holder: String,
        revision: u64,
    },
    /// The refreshed secret, sealed, as the revision after the leased one.
    Refreshed {
        holder: String,
        secret: SealedSecret,
    },
}

/// The directory cell's answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum SecretReply {
    /// Refresh it; write before `until_ms`.
    Leased { until_ms: i64 },
    /// Another executor refreshes it; read again after it writes.
    Held { until_ms: i64 },
    /// The secret is past the revision read: read it again.
    Changed { revision: u64 },
    /// The org has no such secret (removed meanwhile).
    Missing,
    /// The refreshed revision is stored.
    Stored { revision: u64 },
}

/// Apply `op` to `org`'s secrets; a stored refresh goes on the feed.
pub fn apply(
    orgs: &mut Orgs,
    feed: &mut Feed,
    index: u64,
    org: &str,
    op: SecretOp,
    now_ms: i64,
) -> Response {
    let reply = match orgs.orgs.get_mut(org) {
        None => SecretReply::Missing,
        Some(config) if config.deleted => SecretReply::Missing,
        Some(config) => match op {
            SecretOp::Lease {
                secret_id,
                holder,
                revision,
            } => match config.secrets.get(&secret_id) {
                None => SecretReply::Missing,
                Some(current) if current.revision != revision => SecretReply::Changed {
                    revision: current.revision,
                },
                Some(_) => match config.refreshing.get(&secret_id) {
                    Some(lease) if lease.holder != holder && lease.until_ms > now_ms => {
                        SecretReply::Held {
                            until_ms: lease.until_ms,
                        }
                    }
                    _ => {
                        let until_ms = now_ms + LEASE_MS;
                        config
                            .refreshing
                            .insert(secret_id, Lease { holder, until_ms });
                        SecretReply::Leased { until_ms }
                    }
                },
            },
            SecretOp::Refreshed { holder, secret } => {
                let id = secret.secret_id.clone();
                let held = config
                    .refreshing
                    .get(&id)
                    .is_some_and(|l| l.holder == holder);
                match config.secrets.get(&id) {
                    None => SecretReply::Missing,
                    // Not the holder (its lease ran out and another took it),
                    // or not the next revision: whatever is stored stands.
                    Some(current) if !held || secret.revision != current.revision + 1 => {
                        SecretReply::Changed {
                            revision: current.revision,
                        }
                    }
                    Some(_) => {
                        config.refreshing.remove(&id);
                        let revision = secret.revision;
                        let record = Record::SecretRefreshed {
                            secret_id: id.clone(),
                            revision,
                            ciphertext: secret.ciphertext.clone(),
                            nonce: secret.nonce.clone(),
                            data_key_id: secret.data_key_id.clone(),
                        };
                        config.secrets.insert(id, secret);
                        feed.push(index, org, now_ms, record);
                        SecretReply::Stored { revision }
                    }
                }
            }
        },
    };
    Response::Value {
        value: serde_json::to_value(reply).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sealed(revision: u64) -> SealedSecret {
        SealedSecret {
            secret_id: "mcp_oauth:a".into(),
            revision,
            ciphertext: format!("c{revision}"),
            nonce: "n".into(),
            data_key_id: "dk".into(),
        }
    }

    fn orgs() -> Orgs {
        let mut orgs = Orgs::default();
        let config = orgs.orgs.entry("org".into()).or_default();
        config.secrets.insert("mcp_oauth:a".into(), sealed(3));
        orgs
    }

    fn run(orgs: &mut Orgs, feed: &mut Feed, op: SecretOp, now_ms: i64) -> SecretReply {
        match apply(orgs, feed, 1, "org", op, now_ms) {
            Response::Value { value } => serde_json::from_value(value).unwrap(),
            other => panic!("{other:?}"),
        }
    }

    fn lease(holder: &str, revision: u64) -> SecretOp {
        SecretOp::Lease {
            secret_id: "mcp_oauth:a".into(),
            holder: holder.into(),
            revision,
        }
    }

    fn refreshed(holder: &str, revision: u64) -> SecretOp {
        SecretOp::Refreshed {
            holder: holder.into(),
            secret: sealed(revision),
        }
    }

    #[test]
    fn one_executor_refreshes_and_the_rest_read_its_revision() {
        let (mut orgs, mut feed) = (orgs(), Feed::default());
        assert_eq!(
            run(&mut orgs, &mut feed, lease("e1", 3), 1_000),
            SecretReply::Leased {
                until_ms: 1_000 + LEASE_MS
            }
        );
        assert_eq!(
            run(&mut orgs, &mut feed, lease("e2", 3), 2_000),
            SecretReply::Held {
                until_ms: 1_000 + LEASE_MS
            }
        );
        // e2 cannot write over e1's lease.
        assert_eq!(
            run(&mut orgs, &mut feed, refreshed("e2", 4), 2_000),
            SecretReply::Changed { revision: 3 }
        );
        assert_eq!(
            run(&mut orgs, &mut feed, refreshed("e1", 4), 3_000),
            SecretReply::Stored { revision: 4 }
        );
        // e2 now sees the secret moved, and reads revision 4.
        assert_eq!(
            run(&mut orgs, &mut feed, lease("e2", 3), 4_000),
            SecretReply::Changed { revision: 4 }
        );
        assert_eq!(orgs.orgs["org"].secrets["mcp_oauth:a"].revision, 4);
        assert_eq!(feed.records.len(), 1);
        assert!(matches!(
            &feed.records[0].record,
            Record::SecretRefreshed { revision: 4, .. }
        ));
    }

    #[test]
    fn a_dead_refreshers_lease_runs_out_and_its_late_write_is_refused() {
        let (mut orgs, mut feed) = (orgs(), Feed::default());
        run(&mut orgs, &mut feed, lease("dead", 3), 0);
        assert_eq!(
            run(&mut orgs, &mut feed, lease("e2", 3), LEASE_MS + 1),
            SecretReply::Leased {
                until_ms: 2 * LEASE_MS + 1
            }
        );
        assert_eq!(
            run(&mut orgs, &mut feed, refreshed("dead", 4), LEASE_MS + 2),
            SecretReply::Changed { revision: 3 }
        );
        assert_eq!(
            run(&mut orgs, &mut feed, refreshed("e2", 4), LEASE_MS + 3),
            SecretReply::Stored { revision: 4 }
        );
    }

    #[test]
    fn an_older_control_plane_revision_never_replaces_a_refreshed_one() {
        let (mut orgs, mut feed) = (orgs(), Feed::default());
        run(&mut orgs, &mut feed, lease("e1", 3), 0);
        run(&mut orgs, &mut feed, refreshed("e1", 4), 1);
        let update = crate::dp::ConfigUpdate {
            epoch: 0,
            version: 1,
            base_version: 0,
            changes: vec![crate::dp::ConfigChange::Secret(sealed(3)).to_value()],
        };
        orgs.apply("org", &update);
        assert_eq!(orgs.orgs["org"].secrets["mcp_oauth:a"].revision, 4);
    }

    #[test]
    fn a_missing_secret_or_org_is_missing() {
        let (mut orgs, mut feed) = (orgs(), Feed::default());
        let op = SecretOp::Lease {
            secret_id: "other".into(),
            holder: "e1".into(),
            revision: 1,
        };
        assert_eq!(run(&mut orgs, &mut feed, op, 0), SecretReply::Missing);
        let reply = apply(&mut orgs, &mut feed, 1, "nobody", lease("e1", 3), 0);
        assert!(matches!(reply, Response::Value { value } if value["result"] == "missing"));
    }
}
