//! Whether the server skips persisting ephemeral events (stream deltas).

/// What the server last said about ephemeral events, at registration and on
/// each heartbeat.
///
/// Decision: the server reports it rather than the worker guessing from its
/// own NATS_URL. The worker never connects to NATS, and a guess that differs
/// from the server either slows streaming (worker waits needlessly) or drops
/// deltas (worker fires and forgets while the server persists everything).
/// Starts false: until the server answers, the blocking path is always safe.
static SERVER_SKIPS_EPHEMERAL: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Record what the server reported about ephemeral events.
pub(crate) fn set_server_skips_ephemeral(skips: bool) {
    SERVER_SKIPS_EPHEMERAL.store(skips, std::sync::atomic::Ordering::Relaxed);
}

/// Whether the control plane has NATS-backed event delivery, meaning ephemeral
/// events skip PG. When true, the worker can fire-and-forget for deltas.
/// When false, all events go to PG — must use blocking gRPC for correct id/sequence.
pub(crate) fn server_supports_ephemeral_skip() -> bool {
    SERVER_SKIPS_EPHEMERAL.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_what_the_server_reports() {
        set_server_skips_ephemeral(false);
        assert!(!server_supports_ephemeral_skip());
        set_server_skips_ephemeral(true);
        assert!(server_supports_ephemeral_skip());
        set_server_skips_ephemeral(false);
    }
}
