//! A session's storage as a person sees and changes it through the control
//! plane (data plane v2, step 11.6): the Storage tab's keys and secret
//! names, the approval mode switch (a reserved key), and the secrets a
//! person gives a session.
//!
//! Decisions:
//! - The values and secrets stay where the agent's tools read them, in the
//!   session's authority on its cell. The control plane keeps serving the
//!   Storage API for both planes with one code path: it reads them here
//!   (`GET /cell/sessions/{id}/storage`) and changes them as a platform
//!   command (`POST`), so a change is ordered with the session's other
//!   writes and reaches the executor running it.
//! - THREAT[TM-TENANT]: a secret arrives sealed by the control plane with
//!   the org's data key, bound to this session and name
//!   (`dataplane_seal::seal_session_secret`), and is refused otherwise: a
//!   node never stores a person's secret in the open. Listing returns
//!   secret names only.

use everruns_contracts::dataplane::SealedSecret;
use everruns_contracts::typed_id::SessionId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::state::{Change, Response, SessionState};

/// Longest key, secret name or value a person stores here.
const MAX_KEY: usize = 255;
const MAX_VALUE: usize = 256 * 1024;

/// A person's change to the session's storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StorageOp {
    SetValue {
        key: String,
        value: String,
    },
    DeleteValue {
        key: String,
    },
    /// `sealed` is the JSON of the secret sealed for this session and name.
    SetSecret {
        name: String,
        sealed: String,
    },
    DeleteSecret {
        name: String,
    },
}

/// The session's values and secret names (never secret values), with
/// when each was first set and last changed where the cell knows it
/// (`stamps`: `value:<key>` and `secret:<name>`, RFC 3339).
pub fn listed(state: &SessionState) -> Value {
    let authority = &state.authority;
    let time = |ms: i64| {
        chrono::DateTime::from_timestamp_millis(ms)
            .map(|t| t.to_rfc3339())
            .unwrap_or_default()
    };
    let stamps: serde_json::Map<String, Value> = authority
        .stamps
        .iter()
        .map(|(key, s)| {
            (
                key.clone(),
                json!({ "created_at": time(s.created_ms), "updated_at": time(s.updated_ms) }),
            )
        })
        .collect();
    json!({
        "values": authority.values,
        "secrets": authority.secrets.keys().collect::<Vec<_>>(),
        "stamps": stamps,
    })
}

/// The session's secrets as stored, values included: on a data plane with
/// a control plane every one is sealed with the org's data key, and only
/// the control plane reads them, to copy them into a fork and to pause,
/// resume or delete the session's sandbox with the state it keeps in one.
pub fn sealed_secrets(state: &SessionState) -> Value {
    json!({ "secrets": state.authority.secrets })
}

/// Why `op` may not change `session`'s storage, if it may not.
pub fn refused(session: SessionId, op: &StorageOp) -> Option<String> {
    let (key, value) = match op {
        StorageOp::SetValue { key, value } => (key, Some(value)),
        StorageOp::SetSecret { name, sealed } => (name, Some(sealed)),
        StorageOp::DeleteValue { key } | StorageOp::DeleteSecret { name: key } => (key, None),
    };
    if key.trim().is_empty() || key.len() > MAX_KEY {
        return Some("a key is 1 to 255 characters".into());
    }
    if value.is_some_and(|v| v.len() > MAX_VALUE) {
        return Some("a value is at most 256 KiB".into());
    }
    if let StorageOp::SetSecret { name, sealed } = op {
        let bound = serde_json::from_str::<SealedSecret>(sealed).is_ok_and(|s| {
            s.secret_id
                == everruns_contracts::dataplane_seal::session_secret_id(&session.to_string(), name)
        });
        if !bound {
            return Some("a session secret must arrive sealed for this session".into());
        }
    }
    None
}

/// Apply `op` to `session`'s storage.
pub(crate) fn apply(
    state: &mut SessionState,
    session: SessionId,
    op: StorageOp,
    now_ms: i64,
    changes: &mut Vec<Change>,
) -> Response {
    if let Some(reason) = refused(session, &op) {
        return Response::Rejected { reason };
    }
    let authority = &mut state.authority;
    let value = match op {
        StorageOp::SetValue { key, value } => {
            authority.stamp(false, &key, now_ms);
            authority.values.insert(key, value);
            json!(true)
        }
        StorageOp::DeleteValue { key } => {
            authority.unstamp(false, &key);
            json!(authority.values.remove(&key).is_some())
        }
        StorageOp::SetSecret { name, sealed } => {
            authority.stamp(true, &name, now_ms);
            authority.secrets.insert(name, sealed);
            json!(true)
        }
        StorageOp::DeleteSecret { name } => {
            authority.unstamp(true, &name);
            json!(authority.secrets.remove(&name).is_some())
        }
    };
    changes.push(Change::Session(session));
    Response::Value { value }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_must_arrive_sealed_for_its_session_and_name() {
        let session = SessionId::new();
        let key = everruns_contracts::dataplane_seal::random_key();
        let sealed = |s: SessionId, name: &str| {
            everruns_contracts::dataplane_seal::seal_session_secret(
                &key,
                "k1",
                "org-a",
                &s.to_string(),
                name,
                "hunter2",
            )
            .unwrap()
        };
        let set = |name: &str, sealed: String| StorageOp::SetSecret {
            name: name.into(),
            sealed,
        };
        assert_eq!(
            refused(session, &set("TOKEN", sealed(session, "TOKEN"))),
            None
        );
        assert!(refused(session, &set("TOKEN", "hunter2".into())).is_some());
        assert!(refused(session, &set("TOKEN", sealed(SessionId::new(), "TOKEN"))).is_some());
        assert!(refused(session, &set("TOKEN", sealed(session, "OTHER"))).is_some());
        let value = |key: &str| StorageOp::SetValue {
            key: key.into(),
            value: "v".into(),
        };
        assert_eq!(refused(session, &value("k")), None);
        assert!(refused(session, &value(" ")).is_some());
        assert!(refused(session, &value(&"k".repeat(256))).is_some());
    }

    #[test]
    fn listing_names_secrets_without_their_values() {
        let mut state = SessionState::default();
        state.authority.values.insert("k".into(), "v".into());
        state
            .authority
            .secrets
            .insert("TOKEN".into(), "sealed".into());
        let listed = listed(&state);
        assert_eq!(listed["values"]["k"], "v");
        assert_eq!(listed["secrets"], json!(["TOKEN"]));
        assert!(!listed.to_string().contains("sealed"));
    }

    #[test]
    fn sealed_secrets_are_read_as_stored() {
        let mut state = SessionState::default();
        assert_eq!(sealed_secrets(&state), json!({ "secrets": {} }));
        state
            .authority
            .secrets
            .insert("TOKEN".into(), "sealed-token".into());
        assert_eq!(
            sealed_secrets(&state),
            json!({ "secrets": { "TOKEN": "sealed-token" } })
        );
    }

    #[test]
    fn deleting_reports_whether_it_was_there() {
        let session = SessionId::new();
        let mut state = SessionState::default();
        let mut changes = Vec::new();
        let set = StorageOp::SetValue {
            key: "k".into(),
            value: "v".into(),
        };
        assert!(matches!(
            apply(&mut state, session, set, 1_000, &mut changes),
            Response::Value { .. }
        ));
        let delete = || StorageOp::DeleteValue { key: "k".into() };
        let first = apply(&mut state, session, delete(), 2_000, &mut changes);
        let second = apply(&mut state, session, delete(), 2_000, &mut changes);
        assert!(matches!(first, Response::Value { value } if value == json!(true)));
        assert!(matches!(second, Response::Value { value } if value == json!(false)));
    }

    #[test]
    fn a_value_keeps_when_it_was_first_set_and_last_changed() {
        let session = SessionId::new();
        let mut state = SessionState::default();
        let mut changes = Vec::new();
        let set = |value: &str| StorageOp::SetValue {
            key: "k".into(),
            value: value.into(),
        };
        apply(
            &mut state,
            session,
            set("a"),
            1_700_000_000_000,
            &mut changes,
        );
        apply(
            &mut state,
            session,
            set("b"),
            1_700_000_060_000,
            &mut changes,
        );
        let stamp = &listed(&state)["stamps"]["value:k"];
        assert_eq!(stamp["created_at"], "2023-11-14T22:13:20+00:00");
        assert_eq!(stamp["updated_at"], "2023-11-14T22:14:20+00:00");
        let delete = StorageOp::DeleteValue { key: "k".into() };
        apply(&mut state, session, delete, 1_700_000_120_000, &mut changes);
        assert!(
            state.authority.stamps.is_empty(),
            "a removed value's times go with it"
        );
        apply(
            &mut state,
            session,
            set("c"),
            1_700_000_180_000,
            &mut changes,
        );
        assert_eq!(
            listed(&state)["stamps"]["value:k"]["created_at"],
            "2023-11-14T22:16:20+00:00",
            "set again, it starts over"
        );
    }
}
