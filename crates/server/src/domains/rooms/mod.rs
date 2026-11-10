// Room domain: org-owned conversation spaces where people, Everruns agents,
// outside agents and guests talk, with mentions, threads, reactions, pins,
// read marks and a change feed. Agents are woken by what they are addressed
// in and answer through `send_message`.
//
// See knowledge/runtime-resources/rooms.md for the design and its decisions.

use everruns_core::{OrgRole, Policy, Rule};

pub mod commands;
pub mod files;
pub mod listener;
pub mod queries;
pub mod rules;
pub mod types;
pub mod wake;

pub use commands::*;
pub use files::*;
pub use listener::RoomPostListener;

/// Policy: use the org's rooms. Any org member; room-level checks (owners,
/// acting as another member) happen in the commands.
pub const ROOM_ACCESS: Policy = Policy {
    id: "room.access",
    rules: &[Rule::UserHasRole(OrgRole::Member)],
};

#[cfg(test)]
mod tests;
