//! The stored format of shard commands and shard state, pinned.
//!
//! Decision: the write-ahead log, snapshots and the shipped log in the object
//! store hold [`Command`]s and [`ShardState`] as JSON, and a node must read
//! what an earlier release wrote: on a rolling upgrade, a restart, and a
//! restore from the store. Renaming a variant or a field in Rust renames it
//! on disk too. The cell-to-shard rename did that (`register_cell` became
//! `register_shard`, and `shards` changed meaning from directory slots to
//! the shard registry), and a node of the new build could not start on a
//! log the old one wrote (live data plane, 2026-11-22).
//!
//! Each case below is the exact JSON a shard writes today. A change that
//! fails here changes what is on disk: keep the old name readable (a serde
//! alias, where it does not clash) or convert the stored data in the same
//! change, then add the new form here.

use super::*;

const SESSION: &str = "session_00000000000000000000000000000001";

fn session_case(json: &str) -> String {
    json.replace("$S", SESSION)
}

/// Commands whose shape is the shard's own (placement, registry, moves,
/// claims), not the engine's.
const COMMANDS: &[&str] = &[
    r#"{"op":"assign","session":"$S","tenant":"org_1"}"#,
    r#"{"op":"tenant","session":"$S"}"#,
    r#"{"op":"importing","session":"$S","on":true}"#,
    r#"{"op":"feed_acked","through":[7,2]}"#,
    r#"{"op":"reserve","session":"$S","now_ms":5}"#,
    r#"{"op":"claim","session":"$S","holder":"e1","node":2,"ttl_ms":30000,"now_ms":5}"#,
    r#"{"op":"renew","session":"$S","holder":"e1","fence":3,"ttl_ms":30000,"consumed":4,"now_ms":5}"#,
    r#"{"op":"progress","session":"$S","holder":"e1","fence":3,"now_ms":5}"#,
    r#"{"op":"release","session":"$S","holder":"e1","fence":3,"consumed":4,"hand_off":false,"now_ms":5}"#,
    r#"{"op":"place","session":"$S","shard":1,"generation":2,"previous":1}"#,
    r#"{"op":"locate","session":"$S"}"#,
    r#"{"op":"register_shard","shard":1,"members":[[1,"10.0.0.1:7000"],[2,"10.0.0.2:7000"]]}"#,
    r#"{"op":"drain_shard","shard":1,"drain":true}"#,
    r#"{"op":"retire_shard","shard":1}"#,
    r#"{"op":"freeze","session":"$S","to":2}"#,
    r#"{"op":"revoke","session":"$S","now_ms":5}"#,
    r#"{"op":"thaw","session":"$S","generation":3,"to":2}"#,
    r#"{"op":"activate","session":"$S","generation":3}"#,
    r#"{"op":"forward","session":"$S","shard":2,"generation":3}"#,
    r#"{"op":"drop","session":"$S","shard":2,"generation":3}"#,
    r#"{"op":"assign_slot","slot":17,"to":1}"#,
    r#"{"op":"seal_slot","slot":17,"to":1}"#,
    r#"{"op":"adopt","slot":17,"entries":[["$S",{"shard":1,"generation":2}]]}"#,
    r#"{"op":"drop_slot","slot":17}"#,
    r#"{"op":"stored","session":"$S","seg":{"from":1,"to":64}}"#,
    r#"{"op":"batch","commands":[{"op":"locate","session":"$S"}]}"#,
    r#"{"op":"chain","commands":[{"op":"locate","session":"$S"}]}"#,
];

#[test]
fn stored_commands_keep_their_names() {
    for case in COMMANDS {
        let json = session_case(case);
        let command: Command = serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("a stored command no longer reads: {json}: {e}"));
        let written = serde_json::to_string(&command).unwrap();
        assert_eq!(written, json, "a command is written differently now");
    }
}

/// The registry and placement part of a snapshot.
#[test]
fn stored_shard_state_keeps_its_names() {
    let session: SessionId = serde_json::from_value(serde_json::json!(SESSION)).unwrap();
    let mut state = ShardState::default();
    state.shards.insert(1, [(1, "10.0.0.1:7000".to_string())].into());
    state.draining.insert(2);
    state.retired.insert(3);
    state.slots.insert(17, 1);
    state.sealed.insert(18, 2);
    state.directory.insert(
        session,
        Placed {
            shard: 1,
            generation: 2,
        },
    );
    state.forwarded.insert(
        session,
        Left {
            shard: 2,
            generation: 3,
        },
    );
    let written = serde_json::to_value(&state).unwrap();
    let pinned = serde_json::json!({
        "shards": {"1": {"1": "10.0.0.1:7000"}},
        "draining": [2],
        "retired": [3],
        "slots": {"17": 1},
        "sealed": {"18": 2},
        "directory": {SESSION: {"shard": 1, "generation": 2}},
        "forwarded": {SESSION: {"shard": 2, "generation": 3}},
    });
    for (key, value) in pinned.as_object().unwrap() {
        assert_eq!(
            written.get(key),
            Some(value),
            "snapshot field {key} is written differently now"
        );
    }
    let read: ShardState = serde_json::from_value(written).unwrap();
    assert_eq!(read.slots.get(&17), Some(&1));
    assert_eq!(read.shards.get(&1).map(|m| m.len()), Some(1));
}
