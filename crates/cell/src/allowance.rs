//! Spend allowances: a tenant's money a cell may spend without asking.
//!
//! Data plane v2 design, "Budgets stay in the control plane, enforced with
//! allowances", and the `Grants` model (tests/models/data-plane-v2). The
//! control plane grants a cell money from a tenant's balance; the cell
//! spends it locally and reports what each grant used. The guarantee is a
//! strict cap on authorized spend:
//!
//! - Every grant has an id; granting the same id again changes nothing.
//! - Before each provider call the cell reserves the call's maximum cost
//!   from one grant. A call the allowance can't cover does not start: the
//!   Engine's budget gate ends the turn as `budget_exhausted`.
//! - The call's `llm.generation` event settles the reservation at its
//!   actual cost. A cost above the reservation is counted as an overrun.
//! - A reservation whose executor died before the call settled counts as
//!   spent at its maximum (the next epoch's claim closes it): the call may
//!   have run, and money only returns to the balance through a settlement.
//! - An expired grant stops being spendable at once; what it reserved and
//!   used stays until the cell reports it closed.
//! - A metered session in a cell that holds no grant for its tenant (it
//!   moved here, say, before the control plane granted this cell money)
//!   is exhausted, not unmetered: only a session with no tenant runs
//!   without an allowance.
//! - A session that moves out takes no reservation with it: the leaving
//!   copy's open reservation, if its executor died mid-call, closes at its
//!   maximum when the move forwards it.
//!
//! Decision: everything here is replicated cell state, changed only by
//! committed commands, so every replica reports the same usage and a new
//! leader keeps every reservation.
//!
//! Decision: one reservation slot per session. A session runs one provider
//! call at a time, and the Engine's `check_budget` tool also reads through
//! the same check: a second reservation for a session that holds one keeps
//! the one it has instead of doubling it.
//!
//! Grants arrive from the control plane's allowance endpoint (the leader's
//! control loop, `node_control.rs`), or through the cell API in tests that
//! run without one. Not here yet (plan step 7): the per-call maximum is the
//! grant's `per_call_micros`, not the model's price times the prompt and
//! `max_tokens`.

use std::collections::{BTreeMap, HashMap};

use everruns_contracts::typed_id::SessionId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Every tenant's allowance in this cell.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Allowances {
    pub tenants: BTreeMap<String, Allowance>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Allowance {
    pub grants: BTreeMap<String, Grant>,
    /// Open reservations, by session: the grant and the amount.
    pub reserved: HashMap<SessionId, Reservation>,
    /// Allowance requests the control plane answered for this tenant: the
    /// next request's number, so a new leader resends an unanswered one
    /// under the same id.
    #[serde(default)]
    pub requests: u64,
    /// The control plane's last answer granted nothing, for this reason
    /// (`budget_exhausted`, `suspended`, ...): turns run and end exhausted
    /// instead of waiting for money.
    #[serde(default)]
    pub refused: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Grant {
    pub granted_micros: i64,
    /// The most one provider call may cost.
    pub per_call_micros: i64,
    /// After this the cell stops spending from the grant.
    pub spend_until_ms: i64,
    /// Actual cost settled against the grant.
    #[serde(default)]
    pub used_micros: i64,
    /// Cost above what a call reserved.
    #[serde(default)]
    pub overrun_micros: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reservation {
    pub grant: String,
    pub micros: i64,
}

impl Allowances {
    /// Take grant `id` for `tenant`. `false`: already taken.
    pub fn grant(&mut self, tenant: &str, id: &str, grant: Grant) -> bool {
        let allowance = self.tenants.entry(tenant.to_owned()).or_default();
        if allowance.grants.contains_key(id) {
            return false;
        }
        allowance.grants.insert(id.to_owned(), grant);
        true
    }

    /// Reserve the next provider call of `session` for `tenant`. The reply
    /// is the Engine's budget reading (`BudgetToolResponse`).
    pub fn reserve(&mut self, tenant: Option<&str>, session: SessionId, now_ms: i64) -> Value {
        let Some(tenant) = tenant else {
            return json!({ "status": "no_budgets", "budgets": [] });
        };
        let Some(allowance) = self.tenants.get_mut(tenant) else {
            // Metered, but nothing granted to this cell for the tenant.
            return json!({ "status": "exhausted", "budgets": [] });
        };
        if allowance.reserved.contains_key(&session) {
            return allowance.reading("active");
        }
        let pick = allowance
            .grants
            .iter()
            .filter(|(_, g)| g.spend_until_ms > now_ms)
            .find(|(id, g)| allowance.available(id, g) >= g.per_call_micros)
            .map(|(id, g)| (id.clone(), g.per_call_micros));
        match pick {
            Some((grant, micros)) => {
                allowance
                    .reserved
                    .insert(session, Reservation { grant, micros });
                allowance.reading("active")
            }
            None => allowance.reading("exhausted"),
        }
    }

    /// The call `session` reserved finished at `actual_micros`.
    pub fn settle(&mut self, tenant: Option<&str>, session: SessionId, actual_micros: i64) {
        let Some(allowance) = tenant.and_then(|t| self.tenants.get_mut(t)) else {
            return;
        };
        let Some((grant, reserved)) = allowance.take(session) else {
            return;
        };
        if let Some(g) = allowance.grants.get_mut(&grant) {
            g.used_micros += actual_micros.max(0);
            g.overrun_micros += (actual_micros - reserved).max(0);
        }
    }

    /// A new epoch took `session`, or the session left this cell: a
    /// reservation still open belongs to an executor that died mid-call,
    /// and counts as spent at its maximum.
    pub fn close_dangling(&mut self, tenant: Option<&str>, session: SessionId) {
        let Some(allowance) = tenant.and_then(|t| self.tenants.get_mut(t)) else {
            return;
        };
        if let Some((grant, reserved)) = allowance.take(session)
            && let Some(g) = allowance.grants.get_mut(&grant)
        {
            g.used_micros += reserved;
        }
    }
}

impl Allowance {
    /// What the unexpired grants can still reserve.
    pub fn spendable(&self, now_ms: i64) -> i64 {
        self.grants
            .iter()
            .filter(|(_, g)| g.spend_until_ms > now_ms)
            .map(|(id, g)| self.available(id, g).max(0))
            .sum()
    }

    /// What each grant used and holds, for the next allowance request. A
    /// grant is closed once it expired and holds no reservation: nothing
    /// more will be charged to it.
    pub fn settlements(&self, now_ms: i64) -> Vec<crate::dp::GrantSettlement> {
        self.grants
            .iter()
            .map(|(id, g)| {
                let held = self.held(id);
                crate::dp::GrantSettlement {
                    grant_id: id.clone(),
                    used_micros: g.used_micros,
                    reserved_micros: held,
                    closed: g.spend_until_ms <= now_ms && held == 0,
                }
            })
            .collect()
    }

    /// The control plane answered request `request`: take its grant (if
    /// any) and forget the grants that request reported closed. `false`:
    /// that request was answered already.
    pub fn answered(
        &mut self,
        request: u64,
        grant: Option<(String, Grant)>,
        refused: Option<String>,
        closed: &[String],
    ) -> bool {
        if request != self.requests {
            return false;
        }
        self.requests += 1;
        for id in closed {
            if self.held(id) == 0 {
                self.grants.remove(id);
            }
        }
        if let Some((id, grant)) = grant {
            self.grants.entry(id).or_insert(grant);
        }
        self.refused = refused;
        true
    }

    fn held(&self, id: &str) -> i64 {
        self.reserved
            .values()
            .filter(|r| r.grant == id)
            .map(|r| r.micros)
            .sum()
    }

    /// What grant `id` can still reserve.
    fn available(&self, id: &str, grant: &Grant) -> i64 {
        let held: i64 = self
            .reserved
            .values()
            .filter(|r| r.grant == id)
            .map(|r| r.micros)
            .sum();
        grant.granted_micros - grant.used_micros - held
    }

    fn take(&mut self, session: SessionId) -> Option<(String, i64)> {
        let reservation = self.reserved.remove(&session)?;
        Some((reservation.grant, reservation.micros))
    }

    /// The Engine's budget reading: one summary per grant, in dollars.
    fn reading(&self, status: &str) -> Value {
        let budgets: Vec<Value> = self
            .grants
            .iter()
            .map(|(id, g)| {
                let balance = self.available(id, g);
                json!({
                    "budget_id": id,
                    "currency": "usd",
                    "limit": dollars(g.granted_micros),
                    "balance": dollars(balance),
                    "percent_remaining": if g.granted_micros > 0 {
                        100.0 * balance as f64 / g.granted_micros as f64
                    } else {
                        0.0
                    },
                    "status": if balance < g.per_call_micros { "exhausted" } else { "active" },
                })
            })
            .collect();
        json!({ "status": status, "budgets": budgets })
    }
}

fn dollars(micros: i64) -> f64 {
    micros as f64 / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: Option<&str> = Some("t1");

    fn grant(micros: i64, per_call: i64, until: i64) -> Grant {
        Grant {
            granted_micros: micros,
            per_call_micros: per_call,
            spend_until_ms: until,
            used_micros: 0,
            overrun_micros: 0,
        }
    }

    fn status(reading: &Value) -> &str {
        reading["status"].as_str().unwrap()
    }

    #[test]
    fn a_call_starts_only_if_its_maximum_fits() {
        let mut a = Allowances::default();
        assert!(a.grant("t1", "g1", grant(250, 100, 1000)));
        assert!(!a.grant("t1", "g1", grant(9999, 1, 1000)), "same id again");
        let (s1, s2, s3) = (SessionId::new(), SessionId::new(), SessionId::new());
        assert_eq!(status(&a.reserve(T, s1, 0)), "active");
        assert_eq!(status(&a.reserve(T, s2, 0)), "active");
        // 200 reserved of 250: a third call's 100 does not fit.
        assert_eq!(status(&a.reserve(T, s3, 0)), "exhausted");
        // s1's call cost 30: 70 goes back.
        a.settle(T, s1, 30);
        assert_eq!(status(&a.reserve(T, s3, 0)), "active");
        let g = &a.tenants["t1"].grants["g1"];
        assert_eq!((g.used_micros, g.overrun_micros), (30, 0));
    }

    #[test]
    fn a_session_holds_one_reservation() {
        let mut a = Allowances::default();
        a.grant("t1", "g1", grant(150, 100, 1000));
        let s = SessionId::new();
        assert_eq!(status(&a.reserve(T, s, 0)), "active");
        // The check_budget tool reads through the same check.
        assert_eq!(status(&a.reserve(T, s, 0)), "active");
        assert_eq!(a.tenants["t1"].reserved.len(), 1);
    }

    #[test]
    fn a_call_that_died_counts_at_its_maximum() {
        let mut a = Allowances::default();
        a.grant("t1", "g1", grant(200, 100, 1000));
        let s = SessionId::new();
        a.reserve(T, s, 0);
        a.close_dangling(T, s);
        let g = &a.tenants["t1"].grants["g1"];
        assert_eq!(g.used_micros, 100);
        assert!(a.tenants["t1"].reserved.is_empty());
        // A late settlement for the call changes nothing more.
        a.settle(T, s, 40);
        assert_eq!(a.tenants["t1"].grants["g1"].used_micros, 100);
    }

    #[test]
    fn an_expired_grant_stops_spending_but_keeps_its_usage() {
        let mut a = Allowances::default();
        a.grant("t1", "a-old", grant(1000, 100, 50));
        a.grant("t1", "b-new", grant(100, 100, 5000));
        let (s1, s2, s3) = (SessionId::new(), SessionId::new(), SessionId::new());
        // Grants are tried in id order: s1 takes "a-old".
        a.reserve(T, s1, 10);
        assert_eq!(a.tenants["t1"].reserved[&s1].grant, "a-old");
        a.settle(T, s1, 80);
        // After 50 "a-old" is not spendable; "b-new" serves one call.
        a.reserve(T, s2, 60);
        assert_eq!(a.tenants["t1"].reserved[&s2].grant, "b-new");
        assert_eq!(status(&a.reserve(T, s3, 60)), "exhausted");
        // What the expired grant used stays, for its settlement.
        assert_eq!(a.tenants["t1"].grants["a-old"].used_micros, 80);
    }

    #[test]
    fn a_cost_above_the_reservation_is_an_overrun() {
        let mut a = Allowances::default();
        a.grant("t1", "g1", grant(1000, 100, 1000));
        let s = SessionId::new();
        a.reserve(T, s, 0);
        a.settle(T, s, 130);
        let g = &a.tenants["t1"].grants["g1"];
        assert_eq!((g.used_micros, g.overrun_micros), (130, 30));
    }

    #[test]
    fn sessions_without_a_tenant_are_not_reserved() {
        let mut a = Allowances::default();
        let s = SessionId::new();
        assert_eq!(status(&a.reserve(None, s, 0)), "no_budgets");
    }

    #[test]
    fn a_metered_session_with_no_grant_here_is_exhausted() {
        let mut a = Allowances::default();
        let s = SessionId::new();
        assert_eq!(status(&a.reserve(Some("nobody"), s, 0)), "exhausted");
        assert!(a.tenants.is_empty(), "a refusal records nothing");
    }
}
