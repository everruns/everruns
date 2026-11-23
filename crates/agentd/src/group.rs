//! What agentd keeps per group (`install.rs`): a group is the sessions that
//! share one value of the install's group label (`--group-label org`), or
//! the whole install when it names none. It lives in the directory shard,
//! beside the group's agents and secrets, as one more field of its
//! [`OrgConfig`](crate::control::OrgConfig), so every gateway sees the same
//! and a leader change loses nothing.
//!
//! Decisions:
//! - Spend is a ledger of allowances, the same contract the hosted control
//!   plane answers (`/dp/v1/tenants/{t}/allowances`, cp-dp-protocol.md):
//!   shards ask for money, spend it locally and settle what each grant
//!   used. A group with no spend limit is granted whatever a shard asks
//!   for, so an install that sets none never waits on money.
//! - A grant counts against the limit at its full amount until its shard
//!   closes it; then only what it used. So the limit is a strict cap on
//!   what the group can spend, however many shards hold its money.
//! - Answers are kept by request id, so a shard that asks again after a
//!   lost reply gets the same grant, never a second one.
//! - The concurrency limit is a per-shard cap on running turns (a safety
//!   net, like the hosted policy's limits): exact on one shard, and on
//!   many it bounds each shard's share.
//! - The install's controller is recorded once, by the first push; any
//!   other caller is refused (design: one controller per install).

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::dp::{AllowanceGrant, AllowanceRequest};

/// Answers kept for repeated requests.
const ANSWERS_KEPT: usize = 256;
/// How long a shard may spend one grant.
pub const GRANT_FOR_MS: i64 = 60 * 60 * 1000;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Group {
    /// Turns the group runs at once per shard (`None`: no limit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    /// The most the group may spend, ever (`None`: no limit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend_limit_micros: Option<i64>,
    /// What closed grants used.
    #[serde(default)]
    pub spent_closed_micros: i64,
    /// Open grants by id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub grants: BTreeMap<String, Granted>,
    /// Recent answers by request id, oldest first.
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    pub answered: VecDeque<(String, AllowanceGrant)>,
    /// The caller that pushes to this install (on the install's group only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Granted {
    pub amount_micros: i64,
    pub used_micros: i64,
}

/// A change to a group ([`crate::state::Command::Group`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum GroupOp {
    /// Replace the limits.
    Set {
        #[serde(default)]
        max_turns: Option<u32>,
        #[serde(default)]
        spend_limit_micros: Option<i64>,
    },
    /// A shard asking for money and settling its grants.
    Ask { request: AllowanceRequest },
    /// Record the install's controller; refused for another one.
    Claim { controller: String },
}

/// What a group reports (`GET /v1/groups/{value}`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GroupView {
    pub max_turns: Option<u32>,
    pub spend_limit_micros: Option<i64>,
    /// Used by closed grants plus used so far by open ones.
    pub spent_micros: i64,
    /// Granted to shards and not used yet.
    pub reserved_micros: i64,
}

impl Group {
    /// What the group has committed: closed grants' use, open grants whole.
    fn committed(&self) -> i64 {
        self.spent_closed_micros
            + self
                .grants
                .values()
                .map(|g| g.amount_micros.max(g.used_micros))
                .sum::<i64>()
    }

    pub fn view(&self) -> GroupView {
        let used: i64 = self.grants.values().map(|g| g.used_micros).sum();
        GroupView {
            max_turns: self.max_turns,
            spend_limit_micros: self.spend_limit_micros,
            spent_micros: self.spent_closed_micros + used,
            reserved_micros: self.committed() - self.spent_closed_micros - used,
        }
    }

    /// Answer a shard's request, settling its grants first.
    pub fn ask(&mut self, request: &AllowanceRequest, now_ms: i64) -> AllowanceGrant {
        if let Some((_, answer)) = self
            .answered
            .iter()
            .find(|(id, _)| *id == request.request_id)
        {
            return answer.clone();
        }
        for settlement in &request.settlements {
            let Some(grant) = self.grants.get_mut(&settlement.grant_id) else {
                continue;
            };
            grant.used_micros = grant.used_micros.max(settlement.used_micros);
            if settlement.closed {
                self.spent_closed_micros += grant.used_micros;
                self.grants.remove(&settlement.grant_id);
            }
        }
        let available = match self.spend_limit_micros {
            Some(limit) => (limit - self.committed()).max(0),
            None => i64::MAX,
        };
        let granted = request.want_micros.max(0).min(available);
        let answer = AllowanceGrant {
            grant_id: request.request_id.clone(),
            granted_micros: granted,
            spend_until_unix_ms: now_ms + GRANT_FOR_MS,
            reason: (granted == 0 && request.want_micros > 0).then(|| "budget_exhausted".into()),
            per_call_micros: None,
        };
        if granted > 0 {
            self.grants.insert(
                answer.grant_id.clone(),
                Granted {
                    amount_micros: granted,
                    used_micros: 0,
                },
            );
        }
        self.answered
            .push_back((request.request_id.clone(), answer.clone()));
        while self.answered.len() > ANSWERS_KEPT {
            self.answered.pop_front();
        }
        answer
    }
}

/// Apply `op` to `org`'s group (directory shard).
pub fn apply(
    orgs: &mut crate::control::Orgs,
    org: &str,
    op: GroupOp,
    now_ms: i64,
) -> crate::state::Response {
    use crate::state::Response;
    let config = orgs.orgs.entry(org.to_owned()).or_default();
    let group = config.group.get_or_insert_with(Group::default);
    match op {
        GroupOp::Set {
            max_turns,
            spend_limit_micros,
        } => {
            group.max_turns = max_turns;
            group.spend_limit_micros = spend_limit_micros;
            Response::Value {
                value: serde_json::to_value(group.view()).unwrap_or_default(),
            }
        }
        GroupOp::Ask { request } => Response::Value {
            value: serde_json::to_value(group.ask(&request, now_ms)).unwrap_or_default(),
        },
        GroupOp::Claim { controller } => match &group.controller {
            Some(current) if *current == controller => Response::Duplicate,
            Some(current) => Response::Rejected {
                reason: format!("this install takes pushes from {current} only"),
            },
            None => {
                group.controller = Some(controller);
                Response::Ok
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dp::GrantSettlement;

    fn ask(id: &str, want: i64, settlements: Vec<GrantSettlement>) -> AllowanceRequest {
        AllowanceRequest {
            request_id: id.into(),
            shard: "shard-0".into(),
            incarnation: 1,
            want_micros: want,
            settlements,
        }
    }

    fn settle(id: &str, used: i64, closed: bool) -> GrantSettlement {
        GrantSettlement {
            grant_id: id.into(),
            used_micros: used,
            reserved_micros: 0,
            closed,
        }
    }

    #[test]
    fn a_group_with_no_limit_gets_what_it_asks_for() {
        let mut group = Group::default();
        let grant = group.ask(&ask("a", 1_000_000, vec![]), 0);
        assert_eq!(grant.granted_micros, 1_000_000);
        assert_eq!(grant.reason, None);
    }

    #[test]
    fn the_limit_caps_open_grants_whole_and_closed_ones_at_their_use() {
        let mut group = Group {
            spend_limit_micros: Some(1_500_000),
            ..Group::default()
        };
        assert_eq!(
            group.ask(&ask("a", 1_000_000, vec![]), 0).granted_micros,
            1_000_000
        );
        // Only half a dollar is left while the first grant is open.
        assert_eq!(
            group.ask(&ask("b", 1_000_000, vec![]), 0).granted_micros,
            500_000
        );
        let refused = group.ask(&ask("c", 1_000_000, vec![]), 0);
        assert_eq!(refused.granted_micros, 0);
        assert_eq!(refused.reason.as_deref(), Some("budget_exhausted"));
        // The first closes having used 0.2: 0.8 returns to the group.
        let after = group.ask(&ask("d", 1_000_000, vec![settle("a", 200_000, true)]), 0);
        assert_eq!(after.granted_micros, 800_000);
        let view = group.view();
        assert_eq!(view.spent_micros, 200_000);
        assert_eq!(view.reserved_micros, 1_300_000);
    }

    #[test]
    fn asking_again_after_a_lost_reply_gets_the_same_grant() {
        let mut group = Group {
            spend_limit_micros: Some(1_000_000),
            ..Group::default()
        };
        let first = group.ask(&ask("a", 600_000, vec![]), 0);
        let again = group.ask(&ask("a", 600_000, vec![]), 5);
        assert_eq!(first, again);
        assert_eq!(group.view().reserved_micros, 600_000);
    }

    #[test]
    fn one_controller_per_install() {
        let mut orgs = crate::control::Orgs::default();
        let claim = |c: &str| GroupOp::Claim {
            controller: c.into(),
        };
        use crate::state::Response;
        assert!(matches!(
            apply(&mut orgs, "install", claim("app"), 0),
            Response::Ok
        ));
        assert!(matches!(
            apply(&mut orgs, "install", claim("app"), 0),
            Response::Duplicate
        ));
        assert!(matches!(
            apply(&mut orgs, "install", claim("other"), 0),
            Response::Rejected { .. }
        ));
    }
}
