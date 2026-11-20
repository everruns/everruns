//! Moving a running v1 session onto its org's data plane (data plane v2,
//! Phase C, decision 4A): the session keeps its id, record and history, and
//! from then on runs on a cell.
//!
//! Decisions:
//! - Switch first, then import. The move marks the session as the data
//!   plane's ([`MOVING`]) in the same transaction that records the move, so
//!   from that moment no message reaches the platform's workers; the
//!   control plane refuses its messages ("moving, send again") until the
//!   import is done. Only a session with no turn running or waiting is
//!   switched, and the move then waits out a turn that started just before
//!   the switch. A turn waiting on tool results is not moved: its call would
//!   be left without an answer.
//! - The import is what a fork carries (`fork_plane.rs`): the conversation
//!   (the people's and the agent's messages with their text and media; tool
//!   calls and their results stay behind, a seeded call without its result
//!   would be refused by a model provider), the values and secrets, and the
//!   workspace's files. The v1 log itself stays where it is, in the control
//!   plane's `events`, never deleted; the move records where it ends
//!   (`dp_session_moves.from_sequence`).
//! - Bounded: the newest messages that fit one seed call
//!   ([`SEED_BYTES`], [`SEED_MESSAGES`]); a file larger than a cell holds
//!   stays behind. What stayed behind is in the move's report, never
//!   silently dropped.
//! - A bound secret stays behind: a cell has no egress binding yet, and
//!   copying it unbound would widen where it may be sent.
//! - Resumable: every step is retry-safe (the create, a seed only while the
//!   session is empty, files written only where absent, storage set to the
//!   same values), so a move that stopped midway is run again and continues.
//! - The session's own settings go with the create as its layer, as for a
//!   session created on a data plane with them (`sessions::session_layer`);
//!   the capabilities its record holds are the resolved set and stay with
//!   its agent.

use std::time::{Duration, Instant};

use base64::Engine as _;
use everruns_contracts::runtime::config_layer::AgentConfigOverlay;
use everruns_contracts::runtime::session_file::InitialFile;
use everruns_contracts::session_commands;
use everruns_contracts::typed_id::SessionId;
use everruns_core::{INPUT_MESSAGE, OUTPUT_MESSAGE_COMPLETED};
use serde::Serialize;
use serde_json::{Value, json};

use super::session_plane::{SessionPlane, conversation_message};
use super::{ControlPlane, org_public};
use crate::storage::SessionRow;

/// The most a seeded conversation carries, serialized.
pub const SEED_BYTES: usize = 1 << 20;
/// The most messages it carries.
pub const SEED_MESSAGES: usize = 4_000;
/// Files sent in one call, base64 encoded, before a call is sent.
const FILES_BATCH_BYTES: usize = 1 << 20;
/// The largest file a cell holds (`crates/cell/src/api_files.rs`).
const FILE_MAX: i64 = 32 * 1024 * 1024;

/// What a move carried and what stayed behind.
#[derive(Debug, Default, Clone, Serialize, PartialEq)]
pub struct MoveReport {
    pub messages: usize,
    /// The oldest messages left out to fit the seed.
    pub messages_left: usize,
    pub values: usize,
    pub secrets: usize,
    pub files: usize,
    /// What stayed behind, and why.
    pub left_behind: Vec<String>,
    /// The conversation was not seeded: the session already held history
    /// on its cell (a message reached it first).
    pub seed_refused: bool,
}

/// How a move ended.
#[derive(Debug, PartialEq)]
pub enum Moved {
    Done(MoveReport),
    /// Not moved, and why (nothing changed).
    Skipped(String),
}

/// Whether the v1 session `row` runs or waits on a turn now.
async fn busy(cp: &ControlPlane, row: &SessionRow) -> anyhow::Result<Option<String>> {
    match row.status.as_str() {
        "waiting_for_tool_results" => {
            return Ok(Some(
                "a turn is waiting on tool results; answer or cancel it first".into(),
            ));
        }
        "active" => return Ok(Some("a turn is running".into())),
        _ => {}
    }
    let Some(actor) = cp.db.get_session_actor(row.id).await? else {
        return Ok(None);
    };
    let leased = actor.lease_holder.is_some()
        && actor
            .lease_expires_at
            .is_some_and(|at| at > chrono::Utc::now());
    if leased || actor.runnable_since.is_some() || actor.consumed_seq < actor.inbox_seq {
        return Ok(Some("a turn is running or about to".into()));
    }
    Ok(None)
}

/// Move `session` of `org_id` onto its data plane, waiting up to `drain`
/// for a turn that started just before the switch.
pub async fn move_session(
    cp: &ControlPlane,
    org_id: i64,
    session: SessionId,
    drain: Duration,
) -> anyhow::Result<Moved> {
    let org = cp
        .db
        .dp_org(org_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("org {org_id} is not on a data plane"))?;
    let row = cp
        .db
        .get_session(org_id, session)
        .await?
        .ok_or_else(|| anyhow::anyhow!("org {org_id} has no session {session}"))?;
    let harness_id = row
        .harness_id
        .ok_or_else(|| anyhow::anyhow!("session {session} has no harness"))?;
    let public = session.to_string();
    match cp.db.dp_session_move(&public).await? {
        Some(moved) if moved.state == "done" => {
            return Ok(Moved::Skipped("already moved".into()));
        }
        Some(_) => {}
        None => {
            if cp.db.dp_session_on_data_plane(&public).await? {
                return Ok(Moved::Skipped("already on the data plane".into()));
            }
            if let Some(reason) = busy(cp, &row).await? {
                return Ok(Moved::Skipped(reason));
            }
            let (key, _) = crate::domains::agent_definitions::key_for(
                &cp.db,
                org_id,
                harness_id,
                row.agent_id,
            )
            .await?;
            if !cp
                .db
                .dp_begin_session_move(org_id, &public, &org.data_plane_id, &key)
                .await?
            {
                return Ok(Moved::Skipped("already on the data plane".into()));
            }
        }
    }

    // Switched: no new turn starts on v1. Wait out one that started just
    // before the switch.
    let deadline = Instant::now() + drain;
    loop {
        let row = cp
            .db
            .get_session(org_id, session)
            .await?
            .ok_or_else(|| anyhow::anyhow!("session {session} went away"))?;
        match busy(cp, &row).await? {
            None => break,
            Some(reason) if Instant::now() >= deadline => {
                anyhow::bail!(
                    "{session} is switched but {reason}; its messages wait until the move is run again"
                );
            }
            Some(_) => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }

    let mut report = MoveReport::default();
    super::sessions::create_on_plane(
        cp,
        org_id,
        session,
        harness_id,
        row.agent_id,
        &[],
        layer(&row),
    )
    .await?;
    let plane = SessionPlane::with(cp, org_id, session)
        .await?
        .ok_or_else(|| anyhow::anyhow!("{session} is not marked as the data plane's"))?;

    let from_sequence = seed(cp, &plane, session, &mut report).await?;
    storage(cp, &org, &plane, session, &mut report).await?;
    files(cp, &plane, &row, &mut report).await?;

    cp.db
        .dp_finish_session_move(&public, from_sequence, &serde_json::to_value(&report)?)
        .await?;
    Ok(Moved::Done(report))
}

/// The session's own settings, as a layer over its agent.
fn layer(row: &SessionRow) -> Option<AgentConfigOverlay> {
    let layer = AgentConfigOverlay {
        system_prompt: row
            .system_prompt
            .clone()
            .filter(|prompt| !prompt.trim().is_empty()),
        tools: serde_json::from_value(row.tools.clone()).unwrap_or_default(),
        mcp_servers: serde_json::from_value(row.mcp_servers.clone()).unwrap_or_default(),
        network_access: row
            .network_access
            .clone()
            .and_then(|value| serde_json::from_value(value).ok()),
        max_iterations: crate::max_iterations::from_db(row.max_iterations),
        parallel_tool_calls: row.parallel_tool_calls,
        default_model_id: row.model_id,
        ..AgentConfigOverlay::default()
    };
    let empty = layer.system_prompt.is_none()
        && layer.tools.is_empty()
        && layer.mcp_servers.is_empty()
        && layer.network_access.is_none()
        && layer.max_iterations.is_none()
        && layer.parallel_tool_calls.is_none()
        && layer.default_model_id.is_none();
    (!empty).then_some(layer)
}

/// Seed the conversation; the last v1 event read.
async fn seed(
    cp: &ControlPlane,
    plane: &SessionPlane,
    session: SessionId,
    report: &mut MoveReport,
) -> anyhow::Result<Option<i32>> {
    let rows = cp
        .db
        .list_events(session, None, None, &[], &[], None, None)
        .await?;
    let from_sequence = rows.iter().map(|row| row.sequence).max();
    let all: Vec<_> = rows
        .iter()
        .filter(|row| row.event_type == INPUT_MESSAGE || row.event_type == OUTPUT_MESSAGE_COMPLETED)
        .filter_map(|row| conversation_message(&row.event_type, &row.data))
        .collect();
    // The newest that fit.
    let mut bytes = 0;
    let mut kept = Vec::new();
    for message in all.iter().rev() {
        let size = serde_json::to_vec(message)?.len();
        if kept.len() == SEED_MESSAGES || bytes + size > SEED_BYTES {
            break;
        }
        bytes += size;
        kept.push(message.clone());
    }
    kept.reverse();
    report.messages = kept.len();
    report.messages_left = all.len() - kept.len();
    if report.messages_left > 0 {
        report.left_behind.push(format!(
            "the {} oldest messages (the seed holds the newest {})",
            report.messages_left, report.messages
        ));
    }
    if !kept.is_empty() {
        let command = uuid::Uuid::new_v5(&session.uuid(), b"everruns.move");
        let reply = plane
            .command(
                &session_commands::SEED,
                &json!({ "command": command, "messages": kept }),
            )
            .await?;
        // False when the session already held history: a retried move, or
        // a message that reached the cell first.
        report.seed_refused = reply["seeded"] == false;
    }
    Ok(from_sequence)
}

/// Copy the values and secrets; a secret is sealed for the session with the
/// org's data key.
async fn storage(
    cp: &ControlPlane,
    org: &crate::storage::DpOrgRow,
    plane: &SessionPlane,
    session: SessionId,
    report: &mut MoveReport,
) -> anyhow::Result<()> {
    for key in cp.db.list_session_keys(session.uuid()).await? {
        if let Some(row) = cp
            .db
            .get_session_key_value(session.uuid(), &key.key)
            .await?
        {
            plane
                .command(
                    &session_commands::STORAGE,
                    &json!({ "kind": "set_value", "key": row.key, "value": row.value }),
                )
                .await?;
            report.values += 1;
        }
    }
    let secrets = cp.db.list_session_secrets(session.uuid()).await?;
    if secrets.is_empty() {
        return Ok(());
    }
    let data_key = cp.data_key(org)?;
    let org_id = org_public(org.org_id);
    for secret in secrets {
        let Some(row) = cp
            .db
            .get_session_secret(session.uuid(), &secret.name)
            .await?
        else {
            continue;
        };
        if row.binding.is_some() {
            report.left_behind.push(format!(
                "bound secret {}: a cell has no egress binding yet",
                row.name
            ));
            continue;
        }
        let value = cp.encryption()?.decrypt_to_string(&row.value_encrypted)?;
        let sealed = everruns_contracts::dataplane_seal::seal_session_secret(
            &data_key,
            &org.data_key_id,
            &org_id,
            &session.to_string(),
            &row.name,
            &value,
        )
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        plane
            .command(
                &session_commands::STORAGE,
                &json!({ "kind": "set_secret", "name": row.name, "sealed": sealed }),
            )
            .await?;
        report.secrets += 1;
    }
    Ok(())
}

/// Copy the workspace's files, each written only where nothing is yet.
async fn files(
    cp: &ControlPlane,
    plane: &SessionPlane,
    row: &SessionRow,
    report: &mut MoveReport,
) -> anyhow::Result<()> {
    let listed = cp.db.list_all_session_files(row.workspace_id).await?;
    let mut batch: Vec<InitialFile> = Vec::new();
    let mut batch_bytes = 0;
    for info in listed.into_iter().filter(|info| !info.is_directory) {
        if info.size_bytes > FILE_MAX {
            report.left_behind.push(format!(
                "file {} ({} bytes): larger than a cell holds",
                info.path, info.size_bytes
            ));
            continue;
        }
        let Some(file) = cp.db.get_session_file(row.workspace_id, &info.path).await? else {
            continue;
        };
        let content = file.content.unwrap_or_default();
        let (content, encoding) = match String::from_utf8(content) {
            Ok(text) => (text, "text"),
            Err(bytes) => (
                base64::engine::general_purpose::STANDARD.encode(bytes.into_bytes()),
                "base64",
            ),
        };
        if !batch.is_empty() && batch_bytes + content.len() > FILES_BATCH_BYTES {
            send_files(plane, std::mem::take(&mut batch)).await?;
            batch_bytes = 0;
        }
        batch_bytes += content.len();
        batch.push(InitialFile {
            path: file.path,
            content,
            encoding: encoding.into(),
            is_readonly: file.is_readonly,
        });
        report.files += 1;
    }
    if !batch.is_empty() {
        send_files(plane, batch).await?;
    }
    Ok(())
}

async fn send_files(plane: &SessionPlane, files: Vec<InitialFile>) -> anyhow::Result<()> {
    let reply: Value = plane
        .command(
            &session_commands::FS,
            &json!({ "op": "seed", "files": files }),
        )
        .await?;
    if reply["result"] == "failed" {
        anyhow::bail!(
            "the files were not copied: {}",
            reply["message"].as_str().unwrap_or("unknown reason")
        );
    }
    Ok(())
}

/// Move `org_id`'s v1 sessions (not deleted; archived ones too with
/// `archived`), one at a time. The org must
/// already be on its data plane, so its new sessions are created there.
pub async fn move_org(
    cp: &ControlPlane,
    org_id: i64,
    archived: bool,
    drain: Duration,
) -> anyhow::Result<Vec<(SessionId, anyhow::Result<Moved>)>> {
    anyhow::ensure!(
        cp.db.dp_org(org_id).await?.is_some(),
        "org {org_id} is not on a data plane"
    );
    let mut outcomes = Vec::new();
    // By id, after the last one seen: a skipped session stays v1.
    let mut after = uuid::Uuid::nil();
    loop {
        let page = cp.db.dp_v1_sessions(org_id, archived, after, 200).await?;
        let Some(last) = page.last() else {
            return Ok(outcomes);
        };
        after = *last;
        for id in page {
            let session = SessionId::from_uuid(id);
            let outcome = move_session(cp, org_id, session, drain).await;
            outcomes.push((session, outcome));
        }
    }
}
