//! Large payloads by reference: a string over [`OFFLOAD_AT`] in what a
//! session's events carry never enters the Raft log or a node's memory
//! whole.
//!
//! Decisions (memory design, 2026-11-23, step 2):
//! - One threshold everywhere, 256 KB. A pasted screenshot, a file read or
//!   a long tool result is one string; ten 5 MB screenshots then cost ten
//!   references in memory, not 50 MB in every replica's log and resident
//!   events.
//! - The proposer moves each such string to the blob store first and puts
//!   a reference in its place ([`reference`]), so the blob is durable
//!   before the entry that names it. The reference is itself a string, so
//!   it fits whatever typed field held the payload and the reducer copies
//!   it into the event unchanged.
//! - Only what becomes event content is offloaded: a session command's
//!   input, seed and resolutions (`Admit`, `Seed`, `Resolve`,
//!   `ResolveWaits`) and an executor's appended event. A definition or a
//!   setting is read for its meaning, so it stays inline whatever its size.
//! - Readers that hand events out (the shard API, streams, an executor's
//!   context) put the payload back ([`resolve`]); stored segments, moves
//!   and snapshots keep the reference. A session is marked once it holds a
//!   reference (`SessionState::blobs`), so sessions without one are never
//!   scanned.
//! - A reference starts with a NUL character, which no client text the
//!   gateway accepts contains ([`forged`]), so a caller cannot name a blob
//!   it did not write.

use std::sync::atomic::{AtomicU64, Ordering};

use everruns_core::events::Event;
use serde_json::Value;

use crate::state::Command;
use crate::store::Store;

/// Strings longer than this (bytes) go to the blob store.
pub const OFFLOAD_AT: usize = 256 * 1024;

const PREFIX: &str = "\u{0}agentd-blob:";

/// Payloads sent to the blob store instead of the log, and their bytes.
pub static OFFLOADED: AtomicU64 = AtomicU64::new(0);
pub static OFFLOADED_BYTES: AtomicU64 = AtomicU64::new(0);

/// The reference that stands for the blob `sha256` of `len` bytes.
pub fn reference(sha256: &str, len: usize) -> String {
    format!("{PREFIX}{sha256}:{len}")
}

/// The blob a reference names, and its length.
pub fn parse(s: &str) -> Option<(&str, usize)> {
    let (sha256, len) = s.strip_prefix(PREFIX)?.split_once(':')?;
    (sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_hexdigit())).then_some(())?;
    Some((sha256, len.parse().ok()?))
}

/// Whether `value` holds text that looks like a reference: client input
/// that does is refused, so references only come from a proposer.
pub fn forged(value: &Value) -> bool {
    match value {
        Value::String(s) => s.starts_with(PREFIX),
        Value::Array(items) => items.iter().any(forged),
        Value::Object(map) => map.values().any(forged),
        _ => false,
    }
}

/// [`forged`] over encoded JSON.
pub fn forged_text(json: &str) -> bool {
    json.contains(ESCAPED)
}

/// [`PREFIX`] as JSON writes it.
const ESCAPED: &str = "\\u0000agentd-blob:";

/// Counts bytes written without keeping them.
struct Count(usize);

impl std::io::Write for Count {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn encoded_len<T: serde::Serialize>(value: &T) -> usize {
    let mut count = Count(0);
    let _ = serde_json::to_writer(&mut count, value);
    count.0
}

/// Replace every string over [`OFFLOAD_AT`] in `value` with its reference;
/// returns the strings taken, by digest.
fn take_large(value: &mut Value, taken: &mut Vec<(String, Vec<u8>)>) {
    match value {
        Value::String(s) if s.len() > OFFLOAD_AT => {
            use sha2::Digest;
            let bytes = std::mem::take(s).into_bytes();
            let sha256 = hex::encode(sha2::Sha256::digest(&bytes));
            *s = reference(&sha256, bytes.len());
            taken.push((sha256, bytes));
        }
        Value::Array(items) => items.iter_mut().for_each(|v| take_large(v, taken)),
        Value::Object(map) => map.values_mut().for_each(|v| take_large(v, taken)),
        _ => {}
    }
}

/// `field` with its large strings moved to `store`; whether any moved.
async fn offload_field<T>(store: &Store, field: &mut T) -> anyhow::Result<bool>
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    if encoded_len(field) <= OFFLOAD_AT {
        return Ok(false);
    }
    let mut value = serde_json::to_value(&*field)?;
    let mut taken = Vec::new();
    take_large(&mut value, &mut taken);
    if taken.is_empty() {
        return Ok(false);
    }
    for (sha256, bytes) in taken {
        let len = bytes.len() as u64;
        let stored = store.put_blob(bytes).await?;
        debug_assert_eq!(stored, sha256);
        OFFLOADED.fetch_add(1, Ordering::Relaxed);
        OFFLOADED_BYTES.fetch_add(len, Ordering::Relaxed);
    }
    *field = serde_json::from_value(value)?;
    Ok(true)
}

/// `command` with the large strings of what becomes event content moved
/// to the blob store and marked (see the module notes).
pub async fn offload(store: &Store, command: Command) -> anyhow::Result<Command> {
    use everruns::hosting::spi::session_commands::SessionCommand;
    Ok(match command {
        Command::Batch { commands } => Command::Batch {
            commands: offload_all(store, commands).await?,
        },
        Command::Chain { commands } => Command::Chain {
            commands: offload_all(store, commands).await?,
        },
        Command::Host {
            session,
            mut command,
            definition,
            now_ms,
            seed,
            blobs,
        } => {
            let carries = matches!(
                command,
                SessionCommand::Admit { .. }
                    | SessionCommand::Seed { .. }
                    | SessionCommand::Resolve { .. }
                    | SessionCommand::ResolveWaits { .. }
            );
            let moved = carries && offload_field(store, &mut command).await?;
            Command::Host {
                session,
                command,
                definition,
                now_ms,
                seed,
                blobs: blobs || moved,
            }
        }
        Command::Append {
            session,
            fence,
            event_id,
            mut request,
            expected,
            now_ms,
            blobs,
        } => {
            let moved = offload_field(store, &mut request).await?;
            Command::Append {
                session,
                fence,
                event_id,
                request,
                expected,
                now_ms,
                blobs: blobs || moved,
            }
        }
        other => other,
    })
}

fn offload_all(
    store: &Store,
    commands: Vec<Command>,
) -> futures::future::BoxFuture<'_, anyhow::Result<Vec<Command>>> {
    Box::pin(async move {
        let mut out = Vec::with_capacity(commands.len());
        for command in commands {
            out.push(offload(store, command).await?);
        }
        Ok(out)
    })
}

/// The blobs `value`'s references name.
fn references(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => {
            if let Some((sha256, _)) = parse(s) {
                out.push(sha256.to_string());
            }
        }
        Value::Array(items) => items.iter().for_each(|v| references(v, out)),
        Value::Object(map) => map.values().for_each(|v| references(v, out)),
        _ => {}
    }
}

fn put_back(value: &mut Value, blobs: &std::collections::HashMap<String, String>) {
    match value {
        Value::String(s) => {
            if let Some(text) = parse(s).and_then(|(sha256, _)| blobs.get(sha256)) {
                *s = text.clone();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|v| put_back(v, blobs)),
        Value::Object(map) => map.values_mut().for_each(|v| put_back(v, blobs)),
        _ => {}
    }
}

/// Put the payloads `events` reference back in place, read from `store`.
pub async fn resolve(store: &Store, events: &mut [Event]) -> anyhow::Result<()> {
    resolve_each(store, events.iter_mut()).await
}

/// [`resolve`] over shared events: those that reference a blob are
/// replaced by resolved copies.
pub async fn resolve_shared(
    store: &Store,
    events: &mut [std::sync::Arc<Event>],
) -> anyhow::Result<()> {
    for event in events.iter_mut() {
        if holds_reference(event) {
            let mut copy = (**event).clone();
            resolve_each(store, std::iter::once(&mut copy)).await?;
            *event = std::sync::Arc::new(copy);
        }
    }
    Ok(())
}

fn holds_reference(event: &Event) -> bool {
    // A reference is a string starting with NUL, which JSON writes as
    // \u0000: a byte scan finds every candidate.
    serde_json::to_string(event)
        .map(|text| text.contains(ESCAPED))
        .unwrap_or(false)
}

async fn resolve_each(
    store: &Store,
    events: impl Iterator<Item = &mut Event>,
) -> anyhow::Result<()> {
    let mut cache = std::collections::HashMap::new();
    for event in events {
        if !holds_reference(event) {
            continue;
        }
        let mut value = serde_json::to_value(&*event)?;
        let mut names = Vec::new();
        references(&value, &mut names);
        for sha256 in names {
            if cache.contains_key(&sha256) {
                continue;
            }
            let bytes = store.get_blob(&sha256).await?;
            let text = String::from_utf8(bytes.to_vec())
                .map_err(|_| anyhow::anyhow!("blob {sha256} is not text"))?;
            cache.insert(sha256, text);
        }
        put_back(&mut value, &cache);
        *event = serde_json::from_value(value)?;
    }
    Ok(())
}

pub fn write_metrics(w: &mut crate::prom::Writer) {
    w.counter(
        "agentd_payloads_offloaded_total",
        "Payloads over 256 KB sent to the blob store instead of the log.",
        OFFLOADED.load(Ordering::Relaxed) as f64,
    );
    w.counter(
        "agentd_payloads_offloaded_bytes_total",
        "Bytes of those payloads.",
        OFFLOADED_BYTES.load(Ordering::Relaxed) as f64,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_round_trip_and_reject_lookalikes() {
        let sha = "a".repeat(64);
        let r = reference(&sha, 300_000);
        assert_eq!(parse(&r), Some((sha.as_str(), 300_000)));
        assert_eq!(parse("agentd-blob:x:1"), None);
        assert_eq!(parse(&format!("{PREFIX}{}:1", "z".repeat(64))), None);
        assert!(forged(&serde_json::json!({"content": [{"text": r}]})));
        assert!(!forged(&serde_json::json!({"content": "plain"})));
    }

    #[test]
    fn only_strings_over_the_threshold_are_taken() {
        let big = "x".repeat(OFFLOAD_AT + 1);
        let mut value = serde_json::json!({
            "small": "y".repeat(OFFLOAD_AT),
            "parts": [{"image": big.clone()}, {"image": big.clone()}],
        });
        let mut taken = Vec::new();
        take_large(&mut value, &mut taken);
        assert_eq!(taken.len(), 2);
        assert_eq!(value["small"].as_str().unwrap().len(), OFFLOAD_AT);
        let r = value["parts"][0]["image"].as_str().unwrap();
        assert_eq!(parse(r).unwrap().1, OFFLOAD_AT + 1);
        let blobs = taken
            .into_iter()
            .map(|(sha, bytes)| (sha, String::from_utf8(bytes).unwrap()))
            .collect();
        put_back(&mut value, &blobs);
        assert_eq!(value["parts"][1]["image"].as_str().unwrap(), big);
    }
}
