//! The move protocol's decisions, after the `Moves` model
//! (tests/models/data-plane-v2): what each reply means and which command
//! comes next. Pure, so the node's move driver (`node.rs`) and the protocol
//! simulation (`tests/protocol_simulation.rs`) run the same rules.
//!
//! A move of a session leaving cell `own` at generation `g`:
//! 1. import a prepared copy at `g + 1` into the target;
//! 2. place it in the directory over `g` (the ownership transfer);
//! 3. activate the target's copy, then forward the leaving one.
//!
//! A target that refuses the copy abandons the move: the directory takes
//! the session back in `own` at `g + 2`, above anything the move may have
//! installed, and the source thaws at that generation.

use everruns::SessionId;

use crate::state::{CellId, Command, Placed, Response};

/// A freeze toward the cell the session already lives in (a stale view
/// asked for it): undo it, and only it.
pub fn unfreeze(own: CellId, session: SessionId) -> Command {
    Command::Thaw {
        session,
        generation: None,
        to: Some(own),
    }
}

/// What the target's reply to the import means: the entry to place.
pub fn after_import(
    own: CellId,
    to: CellId,
    leaving: u64,
    imported: &Response,
) -> Result<Placed, String> {
    match imported {
        Response::Ok | Response::Duplicate => Ok(Placed {
            cell: to,
            generation: leaving + 1,
        }),
        Response::Rejected { .. } => Ok(Placed {
            cell: own,
            generation: leaving + 2,
        }),
        other => Err(format!("import refused: {other:?}")),
    }
}

/// What the directory's reply to placing `proposed` over `leaving` means:
/// the entry the directory now holds above `leaving`.
pub fn after_place(leaving: u64, proposed: Placed, placed: &Response) -> Result<Placed, String> {
    match placed {
        Response::Ok | Response::Duplicate => Ok(proposed),
        // Another try of this move (an earlier leader's, or one to a
        // different target) already committed: finish that one.
        Response::Placement {
            current: Some(current),
        } if current.generation > leaving => Ok(*current),
        other => Err(format!("place refused: {other:?}")),
    }
}

/// The commands that finish a move the directory committed, in order, each
/// with the cell it goes to. `current` is the entry above `leaving`.
pub fn settle(
    own: CellId,
    session: SessionId,
    leaving: u64,
    current: Placed,
) -> Vec<(CellId, Command)> {
    if current.cell == own {
        return vec![(
            own,
            Command::Thaw {
                session,
                generation: Some(current.generation),
                to: None,
            },
        )];
    }
    let mut steps = Vec::new();
    if current.generation == leaving + 1 {
        steps.push((
            current.cell,
            Command::Activate {
                session,
                generation: current.generation,
            },
        ));
    }
    steps.push((
        own,
        Command::Forward {
            session,
            cell: current.cell,
            generation: leaving + 1,
        },
    ));
    steps
}

/// Whether a step's reply lets the move go on.
pub fn step_done(response: &Response) -> bool {
    matches!(response, Response::Ok | Response::Duplicate)
}

/// A prepared copy at `generation` in cell `own` whose move stalled, given
/// the directory's entry: activate it if the directory committed it, drop
/// it if the directory placed the session elsewhere at its generation or
/// later, and otherwise wait (the move is still running).
pub fn settle_prepared(
    own: CellId,
    session: SessionId,
    generation: u64,
    current: Option<Placed>,
) -> Option<Command> {
    match current {
        Some(p) if p.cell == own && p.generation == generation => Some(Command::Activate {
            session,
            generation,
        }),
        Some(p) if p.cell != own && p.generation >= generation => Some(Command::Drop {
            session,
            cell: p.cell,
            generation: p.generation,
        }),
        _ => None,
    }
}
