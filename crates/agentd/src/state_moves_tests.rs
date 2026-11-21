//! The move rules, one cell's state machine at a time: each test replays a
//! step order the `Moves` model (tests/models/data-plane-v2) explores, with
//! the late and duplicated messages that broke funout's unfenced moves.

use super::*;

const A: CellId = 1;
const B: CellId = 2;
const C: CellId = 3;

fn apply(state: &mut CellState, command: Command) -> Response {
    state.apply(command, &mut Vec::new())
}

fn ok(response: Response) {
    assert!(matches!(response, Response::Ok), "{response:?}");
}

fn refused(response: Response) {
    assert!(
        matches!(
            response,
            Response::Rejected { .. } | Response::Placement { .. }
        ),
        "{response:?}"
    );
}

/// A cell holding `session` live at `generation`.
fn holding(session: SessionId, generation: u64) -> CellState {
    let mut state = CellState::default();
    state.sessions.insert(
        session,
        SessionState {
            generation,
            ..SessionState::default()
        },
    );
    state
}

fn image(generation: u64) -> Box<SessionState> {
    Box::new(SessionState {
        generation,
        placement: Placement::Frozen,
        ..SessionState::default()
    })
}

fn placement(state: &CellState, session: SessionId) -> Option<Placement> {
    state.sessions.get(&session).map(|s| s.placement)
}

fn place(session: SessionId, cell: CellId, generation: u64, previous: u64) -> Command {
    Command::Place {
        session,
        cell,
        generation,
        previous,
    }
}

#[test]
fn a_move_installs_prepared_commits_in_the_directory_then_runs() {
    let session = SessionId::new();
    let mut directory = CellState::default();
    let mut a = holding(session, 1);
    let mut b = CellState::default();
    ok(apply(&mut directory, place(session, A, 1, 0)));

    ok(apply(
        &mut a,
        Command::Freeze {
            session,
            to: Some(B),
        },
    ));
    ok(apply(
        &mut b,
        Command::Import {
            session,
            image: image(2),
        },
    ));
    assert_eq!(placement(&b, session), Some(Placement::Prepared));
    // Prepared runs nothing: no claim, no input.
    let claim = Command::Claim {
        session,
        holder: "x".into(),
        node: 1,
        ttl_ms: 1000,
        now_ms: 0,
    };
    assert!(matches!(apply(&mut b, claim), Response::Frozen));

    ok(apply(&mut directory, place(session, B, 2, 1)));
    ok(apply(
        &mut b,
        Command::Activate {
            session,
            generation: 2,
        },
    ));
    ok(apply(
        &mut a,
        Command::Forward {
            session,
            cell: B,
            generation: 2,
        },
    ));
    assert_eq!(placement(&b, session), Some(Placement::Live));
    assert!(placement(&a, session).is_none());
    assert_eq!(
        a.forwarded[&session],
        Left {
            cell: B,
            generation: 1
        }
    );

    // Every step again, as a new leader re-running the move: no change.
    for command in [
        Command::Import {
            session,
            image: image(2),
        },
        Command::Activate {
            session,
            generation: 2,
        },
    ] {
        assert!(matches!(apply(&mut b, command), Response::Duplicate));
    }
    let forward = Command::Forward {
        session,
        cell: B,
        generation: 2,
    };
    assert!(matches!(apply(&mut a, forward), Response::Duplicate));
    assert!(matches!(
        apply(&mut directory, place(session, B, 2, 1)),
        Response::Duplicate
    ));
}

/// funout at 8ce744b: the session went A to B to A; a delayed import of the
/// first move reached B again after B forwarded, and B ran a stale copy.
#[test]
fn a_late_import_cannot_revive_a_copy_that_left() {
    let session = SessionId::new();
    let mut b = CellState::default();
    ok(apply(
        &mut b,
        Command::Import {
            session,
            image: image(2),
        },
    ));
    ok(apply(
        &mut b,
        Command::Activate {
            session,
            generation: 2,
        },
    ));
    ok(apply(
        &mut b,
        Command::Freeze {
            session,
            to: Some(A),
        },
    ));
    ok(apply(
        &mut b,
        Command::Forward {
            session,
            cell: A,
            generation: 3,
        },
    ));

    refused(apply(
        &mut b,
        Command::Import {
            session,
            image: image(2),
        },
    ));
    assert!(placement(&b, session).is_none());
    // A genuinely newer move back is taken.
    ok(apply(
        &mut b,
        Command::Import {
            session,
            image: image(4),
        },
    ));
}

#[test]
fn the_directory_commits_a_generation_only_over_the_one_before() {
    let session = SessionId::new();
    let mut directory = CellState::default();
    ok(apply(&mut directory, place(session, A, 1, 0)));
    ok(apply(&mut directory, place(session, B, 2, 1)));
    ok(apply(&mut directory, place(session, A, 3, 2)));
    // The first move's placement, delayed: it must not repoint the session.
    let late = apply(&mut directory, place(session, B, 2, 1));
    assert!(
        matches!(
            late,
            Response::Placement {
                current: Some(Placed {
                    cell: A,
                    generation: 3
                })
            }
        ),
        "{late:?}"
    );
    // A second first placement of a known session does not move it.
    refused(apply(&mut directory, place(session, C, 1, 0)));
    assert_eq!(
        directory.directory[&session],
        Placed {
            cell: A,
            generation: 3
        }
    );
}

#[test]
fn a_late_forward_never_drops_the_copy_that_runs_now() {
    let session = SessionId::new();
    // A moved the session to B (generation 2), got it back (3), and now
    // leaves again: the forward for generation 2 arrives late.
    let mut a = holding(session, 3);
    refused(apply(
        &mut a,
        Command::Forward {
            session,
            cell: B,
            generation: 2,
        },
    ));
    assert_eq!(placement(&a, session), Some(Placement::Live));
    ok(apply(
        &mut a,
        Command::Freeze {
            session,
            to: Some(C),
        },
    ));
    refused(apply(
        &mut a,
        Command::Forward {
            session,
            cell: B,
            generation: 2,
        },
    ));
    assert_eq!(placement(&a, session), Some(Placement::Frozen));
}

/// The draft with one "frozen" state: a late duplicate activation woke a
/// copy that had already started leaving again.
#[test]
fn activation_wakes_only_a_prepared_copy() {
    let session = SessionId::new();
    let mut b = CellState::default();
    ok(apply(
        &mut b,
        Command::Import {
            session,
            image: image(2),
        },
    ));
    ok(apply(
        &mut b,
        Command::Activate {
            session,
            generation: 2,
        },
    ));
    ok(apply(
        &mut b,
        Command::Freeze {
            session,
            to: Some(C),
        },
    ));
    assert!(matches!(
        apply(
            &mut b,
            Command::Activate {
                session,
                generation: 2
            }
        ),
        Response::Duplicate
    ));
    assert_eq!(placement(&b, session), Some(Placement::Frozen));
    refused(apply(
        &mut b,
        Command::Activate {
            session,
            generation: 5,
        },
    ));
}

#[test]
fn an_abandoned_move_takes_the_session_back_above_the_move() {
    let session = SessionId::new();
    let mut directory = CellState::default();
    let mut a = holding(session, 1);
    let mut b = CellState::default();
    ok(apply(&mut directory, place(session, A, 1, 0)));
    ok(apply(
        &mut a,
        Command::Freeze {
            session,
            to: Some(B),
        },
    ));
    ok(apply(
        &mut b,
        Command::Import {
            session,
            image: image(2),
        },
    ));

    // A gives up before the directory commits B; it takes the session
    // back at 3, so the move's own placement can no longer land.
    ok(apply(&mut directory, place(session, A, 3, 1)));
    ok(apply(
        &mut a,
        Command::Thaw {
            session,
            generation: Some(3),
            to: None,
        },
    ));
    refused(apply(&mut directory, place(session, B, 2, 1)));
    assert_eq!(placement(&a, session), Some(Placement::Live));
    assert_eq!(a.sessions[&session].generation, 3);

    // B's copy can never run; asked, the directory says drop it.
    refused(apply(
        &mut b,
        Command::Activate {
            session,
            generation: 3,
        },
    ));
    ok(apply(
        &mut b,
        Command::Drop {
            session,
            cell: A,
            generation: 3,
        },
    ));
    assert!(placement(&b, session).is_none());
    assert_eq!(
        b.forwarded[&session],
        Left {
            cell: A,
            generation: 2
        }
    );
    refused(apply(
        &mut b,
        Command::Import {
            session,
            image: image(2),
        },
    ));
}

#[test]
fn a_live_copy_is_never_dropped_as_stale() {
    let session = SessionId::new();
    let mut b = CellState::default();
    ok(apply(
        &mut b,
        Command::Import {
            session,
            image: image(2),
        },
    ));
    ok(apply(
        &mut b,
        Command::Activate {
            session,
            generation: 2,
        },
    ));
    refused(apply(
        &mut b,
        Command::Drop {
            session,
            cell: A,
            generation: 9,
        },
    ));
    assert_eq!(placement(&b, session), Some(Placement::Live));
}

#[test]
fn a_shard_hand_over_keeps_the_newer_placement() {
    let session = SessionId::new();
    let mut owner = CellState::default();
    ok(apply(&mut owner, place(session, B, 2, 0)));
    ok(apply(
        &mut owner,
        Command::Adopt {
            shard: shard_of(session),
            entries: vec![(
                session,
                Placed {
                    cell: A,
                    generation: 1,
                },
            )],
        },
    ));
    assert_eq!(
        owner.directory[&session],
        Placed {
            cell: B,
            generation: 2
        }
    );
}

#[test]
fn a_running_move_is_never_retargeted() {
    // Found by the protocol simulation: the directory committed A's move
    // to B, then a late freeze "to A" retargeted the leaving copy, and the
    // next driver thawed it in A while B ran the session.
    let session = SessionId::new();
    let mut a = holding(session, 1);
    let to = |cell| Command::Freeze {
        session,
        to: Some(cell),
    };
    ok(apply(&mut a, to(B)));
    assert!(matches!(apply(&mut a, to(B)), Response::Duplicate));
    assert!(matches!(apply(&mut a, to(A)), Response::Frozen));
    assert!(matches!(apply(&mut a, to(C)), Response::Frozen));
    assert_eq!(a.sessions[&session].moving_to, Some(B));
}
