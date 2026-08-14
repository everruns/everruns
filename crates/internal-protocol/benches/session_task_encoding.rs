// EVE-642 evidence bench: measure the session-task RPC payload encode/decode
// cost for the OLD JSON-in-protobuf path vs the NEW native-protobuf path.
//
// The RPC hot path serializes a `SessionTask` on the server and deserializes it
// on the worker for every task/message, per element on list handlers. The OLD
// path did `serde_json::to_vec` of the whole struct into a protobuf `bytes`
// field (JSON encode) then `serde_json::from_slice` on the worker (JSON decode)
// — JSON work layered on top of protobuf's own framing. The NEW path models the
// ~20 structural fields as native protobuf and lets prost encode/decode them
// once; only the genuinely free-form fields (spec / expected / message Data)
// carry canonical serde_json bytes, serialized exactly once.
//
// This bench isolates the serialization work only (no network), which is exactly
// the redundant CPU/allocation the migration removes. Run:
//   cargo bench -p everruns-internal-protocol --bench session_task_encoding
//
// harness = false: plain `fn main`, matching the crate/durable bench convention.

use std::hint::black_box;
use std::time::Instant;

use chrono::{TimeZone, Utc};
use everruns_core::session_task as st;
use everruns_internal_protocol::{proto_to_session_task, session_task_to_proto};
use everruns_provider::typed_id::SessionId;
use prost::Message as _;

fn sample_task() -> st::SessionTask {
    st::SessionTask {
        id: "task_0123456789abcdef".to_string(),
        session_id: SessionId::new(),
        root_session_id: None,
        kind: st::TASK_KIND_SUBAGENT.to_string(),
        display_name: "Investigate CI flake in worker integration suite".to_string(),
        spec: serde_json::json!({
            "instructions": "Reproduce the flake and propose a fix",
            "budget": { "steps": 40, "tokens": 200_000 },
            "tags": ["ci", "flake", "worker"],
            "context": {
                "branch": "main",
                "files": ["a.rs", "b.rs", "c.rs", "d.rs"],
                "nested": { "deep": { "deeper": [1, 2, 3, 4, 5] } }
            }
        }),
        state: st::SessionTaskState::Running,
        state_detail: Some("iteration 7/40".to_string()),
        progress: Some(st::TaskProgress {
            current: Some(7),
            total: Some(40),
            unit: Some("steps".to_string()),
            label: Some("running tests".to_string()),
        }),
        input_request: None,
        cancel_requested_at: None,
        summary: None,
        result_path: Some("/.tasks/task_0123456789abcdef/result.json".to_string()),
        artifacts: vec![st::TaskArtifact {
            name: "log".to_string(),
            artifact_type: "file".to_string(),
            path: Some("/.tasks/task_0123456789abcdef/log.txt".to_string()),
            url: None,
        }],
        error: None,
        attempt: 1,
        worker_id: Some("worker-node-3".to_string()),
        heartbeat_at: Some(Utc.timestamp_opt(1_700_000_200, 0).unwrap()),
        links: st::TaskLinks {
            child_session_id: Some(SessionId::new()),
            remote_task_id: None,
            resource_ids: vec!["res_sandbox_1".to_string()],
        },
        wake_policy: st::TaskWakePolicy::OnActivity,
        created_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
        started_at: Some(Utc.timestamp_opt(1_700_000_050, 0).unwrap()),
        finished_at: None,
        updated_at: Utc.timestamp_opt(1_700_000_300, 0).unwrap(),
    }
}

/// OLD path: domain struct -> JSON bytes (server) -> JSON bytes -> domain (worker).
/// This is the `serde_json::to_vec` / `from_slice` into a protobuf bytes field.
fn old_round_trip(task: &st::SessionTask) -> st::SessionTask {
    let bytes = serde_json::to_vec(task).expect("json encode");
    serde_json::from_slice(&bytes).expect("json decode")
}

/// NEW path: domain struct -> native proto -> protobuf bytes (framing) -> proto -> domain.
/// prost encodes/decodes once; no JSON layer.
fn new_round_trip(task: &st::SessionTask) -> st::SessionTask {
    let proto = session_task_to_proto(task);
    let mut buf = Vec::with_capacity(proto.encoded_len());
    proto.encode(&mut buf).expect("proto encode");
    let decoded = everruns_internal_protocol::proto::SessionTaskProto::decode(buf.as_slice())
        .expect("decode");
    proto_to_session_task(decoded).expect("proto -> domain")
}

fn bench(name: &str, iters: u64, mut f: impl FnMut()) {
    // Warm up.
    for _ in 0..(iters / 10).max(1) {
        f();
    }
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let elapsed = start.elapsed();
    let per = elapsed.as_nanos() as f64 / iters as f64;
    println!("{name:<28} {iters:>8} iters  {elapsed:>10.3?}  {per:>10.1} ns/op");
}

fn main() {
    let task = sample_task();

    // Report the wire-size difference (protobuf is more compact than JSON text).
    let json_len = serde_json::to_vec(&task).unwrap().len();
    let proto = session_task_to_proto(&task);
    let proto_len = proto.encoded_len();
    println!("payload size:  json bytes = {json_len}, protobuf bytes = {proto_len}");
    println!();

    // Simulate a list handler returning N elements per RPC: this is where the
    // per-element re-encode dominates on the old path.
    const LIST_N: usize = 50;
    let iters: u64 = 20_000;

    bench("old json (single)", iters, || {
        black_box(old_round_trip(black_box(&task)));
    });
    bench("new native-proto (single)", iters, || {
        black_box(new_round_trip(black_box(&task)));
    });

    let list_iters = iters / LIST_N as u64;
    bench("old json (list x50)", list_iters, || {
        for _ in 0..LIST_N {
            black_box(old_round_trip(black_box(&task)));
        }
    });
    bench("new native-proto (list x50)", list_iters, || {
        for _ in 0..LIST_N {
            black_box(new_round_trip(black_box(&task)));
        }
    });
}
