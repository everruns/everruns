//! Group commit at the proposal level.
//!
//! Proposals that reach the leader while a Raft write is in flight are
//! gathered into one `Command::Batch` entry, so one quorum fsync covers many
//! sessions' steps. Up to [`IN_FLIGHT`] batches are pipelined.

use std::sync::Arc;

use tokio::sync::{Semaphore, mpsc, oneshot};

use crate::state::{Command, Response};
use crate::types::Raft;

const MAX_BATCH: usize = 1024;

/// Bytes of commands one entry may carry.
///
/// Decision: openraft gives each AppendEntries one heartbeat interval
/// (100 ms) and resends the same entries after a timeout. An entry is never
/// split, so one that cannot ship and fsync in an interval on a loaded host
/// stalls replication for good: commits stop, claims expire, turns restart,
/// and the load that made the entry big grows. Seen at 75 000 sessions with
/// 1 024 appends in one entry. Entries stay small instead; the WAL still
/// group-commits several of them per fsync.
const MAX_BATCH_BYTES: usize = 64 * 1024;

/// Raft writes in flight at once.
///
/// Decision: 2, not 16. openraft's leader core appends one entry at a time
/// and waits for its flush, so with 16 batches in flight each batch held a
/// handful of commands and the leader fsynced once per entry. With 2, the
/// commands that arrive during a write wait for the next one and share its
/// entry. 40 sessions x 1 000 chat turns: leader fsyncs 410k -> 149k,
/// 101 -> 134 turns/s, turn p50 181 -> 84 ms. 1 was no better than 2.
const IN_FLIGHT: usize = 2;

/// A command whose JSON is longer than this goes to Raft in pieces
/// (`Command::Part`). Escaped inside a piece it can grow to about twice
/// this, which still fits one entry of [`MAX_BATCH_BYTES`].
///
/// Decision: a single 32 MB message as one entry wedged its cell for good:
/// each AppendEntries timed out after 100 ms and was resent, forever.
const PART_BYTES: usize = 32 * 1024;

type Reply = oneshot::Sender<Result<(Response, u64), String>>;

#[derive(Clone)]
pub struct Batcher {
    tx: mpsc::UnboundedSender<(Command, usize, Reply)>,
}

impl Batcher {
    pub fn spawn(raft: Raft) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<(Command, usize, Reply)>();
        tokio::spawn(async move {
            let permits = Arc::new(Semaphore::new(IN_FLIGHT));
            while let Some(first) = rx.recv().await {
                let waited = std::time::Instant::now();
                // The semaphore is never closed, so acquiring cannot fail.
                let Ok(permit) = permits.clone().acquire_owned().await else {
                    break;
                };
                crate::prof::add("permit_wait", waited, 1);
                // Everything that arrived while we waited for a permit joins.
                let mut bytes = first.1;
                let mut batch = vec![(first.0, first.2)];
                while batch.len() < MAX_BATCH && bytes < MAX_BATCH_BYTES {
                    match rx.try_recv() {
                        Ok((command, size, reply)) => {
                            bytes += size;
                            batch.push((command, reply));
                        }
                        Err(_) => break,
                    }
                }
                let raft = raft.clone();
                crate::prof::add(
                    "batch_commands",
                    std::time::Instant::now(),
                    batch.len() as u64,
                );
                tokio::spawn(async move {
                    let (commands, replies): (Vec<_>, Vec<_>) = batch.into_iter().unzip();
                    let single = commands.len() == 1;
                    let command = match <[Command; 1]>::try_from(commands) {
                        Ok([command]) => command,
                        Err(commands) => Command::Batch { commands },
                    };
                    let t = std::time::Instant::now();
                    let written = raft.client_write(command).await;
                    crate::prof::add("raft_write", t, 1);
                    match written {
                        Ok(written) => {
                            let index = written.log_id.index;
                            match (single, written.data) {
                                (false, Response::Batch { responses }) => {
                                    for (reply, response) in replies.into_iter().zip(responses) {
                                        let _ = reply.send(Ok((response, index)));
                                    }
                                }
                                (_, response) => {
                                    for reply in replies {
                                        let _ = reply.send(Ok((response.clone(), index)));
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            let error = error.to_string();
                            for reply in replies {
                                let _ = reply.send(Err(error.clone()));
                            }
                        }
                    }
                    drop(permit);
                });
            }
        });
        Self { tx }
    }

    /// Write `command`, in pieces if it is large, and return its response
    /// and the log index that committed it.
    pub async fn submit(&self, command: Command) -> anyhow::Result<(Response, u64)> {
        let json = serde_json::to_string(&command)?;
        if json.len() <= PART_BYTES {
            return self.submit_one(command, json.len()).await;
        }
        let pieces = split(&json, PART_BYTES);
        let id = uuid::Uuid::new_v4();
        let total = pieces.len() as u32;
        let at_ms = crate::node::now_ms();
        let mut last = None;
        // One after another: two pieces in flight could land out of order.
        for (seq, data) in pieces.into_iter().enumerate() {
            let part = Command::Part {
                id,
                seq: seq as u32,
                total,
                at_ms,
                data: data.to_string(),
            };
            let size = data.len() + 128;
            let (response, index) = self.submit_one(part, size).await?;
            if let Response::Rejected { reason } = &response
                && seq + 1 < total as usize
            {
                anyhow::bail!("piece {seq} of a large command: {reason}");
            }
            last = Some((response, index));
        }
        last.ok_or_else(|| anyhow::anyhow!("a large command split into no pieces"))
    }

    async fn submit_one(&self, command: Command, size: usize) -> anyhow::Result<(Response, u64)> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send((command, size, reply))
            .map_err(|_| anyhow::anyhow!("batcher stopped"))?;
        rx.await
            .map_err(|_| anyhow::anyhow!("batcher dropped the proposal"))?
            .map_err(|error| anyhow::anyhow!(error))
    }
}

/// `text` cut into pieces of at most `max` bytes, on character boundaries.
fn split(text: &str, max: usize) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let mut end = rest.len().min(max);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        let (piece, tail) = rest.split_at(end);
        pieces.push(piece);
        rest = tail;
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_keeps_characters_whole() {
        let text = "aé".repeat(1000);
        let pieces = split(&text, 7);
        assert!(pieces.iter().all(|p| p.len() <= 7 && !p.is_empty()));
        assert_eq!(pieces.concat(), text);
    }
}
