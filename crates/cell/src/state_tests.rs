//! State machine unit tests: pieces, compaction ranges, checkpoints.

use super::*;

#[test]
fn a_command_in_pieces_applies_once_whole() {
    let create = Command::RegisterCell {
        cell: 9,
        members: vec![(1, "x".repeat(5000))],
    };
    let json = serde_json::to_string(&create).unwrap();
    let id = uuid::Uuid::new_v4();
    let pieces: Vec<String> = json
        .as_bytes()
        .chunks(1000)
        .map(|c| String::from_utf8(c.to_vec()).unwrap())
        .collect();
    let total = pieces.len() as u32;
    let mut state = CellState::default();
    let mut changes = Vec::new();
    for (seq, data) in pieces.iter().enumerate() {
        let response = state.apply(
            Command::Part {
                id,
                seq: seq as u32,
                total,
                at_ms: 1,
                data: data.clone(),
            },
            &mut changes,
        );
        if seq + 1 < total as usize {
            assert!(matches!(response, Response::Ok));
            assert!(!state.cells.contains_key(&9));
        } else {
            assert!(matches!(response, Response::Ok), "{response:?}");
        }
    }
    assert!(state.cells.contains_key(&9));
    assert!(state.parts.is_empty());

    // A piece out of order drops the assembly; a stale one is collected.
    let stale = uuid::Uuid::new_v4();
    let part = |id, seq, at_ms| Command::Part {
        id,
        seq,
        total: 3,
        at_ms,
        data: String::new(),
    };
    state.apply(part(stale, 0, 1), &mut changes);
    assert!(matches!(
        state.apply(part(id, 1, 1), &mut changes),
        Response::Rejected { .. }
    ));
    state.apply(part(id, 0, 1 + PART_TTL_MS), &mut changes);
    assert_eq!(state.parts.keys().collect::<Vec<_>>(), vec![&id]);
}

/// Commands travel as internally tagged JSON (Raft entries, proposals);
/// every shape must survive the round trip.
#[test]
fn compaction_keeps_tiers_whatever_the_flush_sizes() {
    use crate::store::SegRef;
    let mut s = SessionState::default();
    // Flushes of 11 to 77 events (a turn or a busy tail); compact
    // whenever 16 segments pile up.
    let mut writes: i64 = 0;
    let mut rng: u64 = 7;
    for flush in 0..6000 {
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let n = 11 * (1 + (rng >> 33) % 7) as i32;
        let from = s.flushed() + 1;
        s.record(SegRef {
            from,
            to: from + n - 1,
            inc: 0,
        });
        writes += i64::from(n);
        if s.segments.len() >= 16
            && let Some((from, to)) = s.compaction_range()
        {
            writes += i64::from(to - from + 1);
            s.record(SegRef { from, to, inc: 0 });
        }
        assert!(s.segments.len() <= 20, "flush {flush}: {:?}", s.segments);
        // Contiguous from 1; the first segment is the first flush.
        assert_eq!(s.segments[0].from, 1);
        assert!(s.segments[0].to <= 77);
        assert!(s.segments.windows(2).all(|w| w[1].from == w[0].to + 1));
    }
    // Each event rewritten a handful of times, not once per compaction.
    let events = i64::from(s.flushed());
    assert!(writes < events * 10, "wrote {writes} for {events} events");
    // A segment that matches no run, or starts nowhere recorded, is ignored.
    let before = s.segments.clone();
    s.record(SegRef {
        from: 7,
        to: s.flushed(),
        inc: 0,
    });
    s.record(SegRef {
        from: 1,
        to: 5,
        inc: 0,
    });
    assert_eq!(s.segments, before);
}

#[test]
fn register_cell_round_trips() {
    let command = Command::RegisterCell {
        cell: 3,
        members: vec![(1, "127.0.0.1:7301".into()), (4, "127.0.0.1:7304".into())],
    };
    let json = serde_json::to_string(&command).unwrap();
    let back: Command = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, Command::RegisterCell { cell: 3, members } if members.len() == 2));
}

/// A session keeps its newest checkpoint per model, written only under
/// the current epoch.
#[test]
fn checkpoints_keep_the_newest_per_model() {
    let session = SessionId::new();
    let mut cell = CellState::default();
    let mut state = SessionState::default();
    state.authority.fence = 7;
    state.authority.holder = Some(crate::authority::Holder {
        name: "h".into(),
        node: 1,
        fence: 7,
        expires_ms: 1,
    });
    cell.sessions.insert(session, state);
    let checkpoint = |model: &str, at: i64| Checkpoint {
        id: uuid::Uuid::new_v4(),
        source_sequence: at,
        provider_type: "llmsim".into(),
        model: model.into(),
        format_version: 1,
        payload: everruns_core::CompactionCheckpointPayload::Summary {
            text: format!("up to {at}"),
        },
    };
    let mut apply = |fence, c| {
        let command = Command::Write {
            session,
            fence,
            change: crate::authority::WriteOp::Checkpoint { checkpoint: c },
            now_ms: 0,
        };
        let command: Command =
            serde_json::from_str(&serde_json::to_string(&command).unwrap()).unwrap();
        match cell.apply(command, &mut Vec::new()) {
            Response::Value { value } => Some(value.as_bool().unwrap()),
            Response::Fenced => None,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(apply(7, checkpoint("a", 10)), Some(true));
    assert_eq!(apply(7, checkpoint("a", 5)), Some(false));
    assert_eq!(apply(7, checkpoint("b", 3)), Some(true));
    assert_eq!(apply(7, checkpoint("a", 20)), Some(true));
    assert_eq!(apply(6, checkpoint("a", 30)), None);
    let kept = &cell.sessions[&session].checkpoints;
    assert_eq!(kept.len(), 2);
    assert_eq!(
        kept.iter()
            .find(|c| c.model == "a")
            .unwrap()
            .source_sequence,
        20
    );
}
