//! A shard node binary with simulated models and test tools (tests only).
//! Agents come from the clients.

// Decision: jemalloc, not the system allocator or mimalloc. Under glibc
// malloc the Raft leader (many threads writing segments, snapshots and turns
// at once) held three times its live heap: 1.3 GB of arenas for 420 MB in use
// after 40k turns, which malloc_trim does not return. mimalloc did better but
// kept freed memory too: 5 400 sessions after a 16 000-turn run held 1.2 GB
// with no events loaded, against 400 MB for the same node restarted. jemalloc
// with a background thread purging freed pages after a second settled at
// 750 MB and gives memory back as load falls (funout measured the same).
#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// jemalloc's options (a C string; `_RJEM_MALLOC_CONF` overrides them).
#[allow(non_upper_case_globals)]
#[unsafe(export_name = "_rjem_malloc_conf")]
pub static malloc_conf: Option<&u8> =
    Some(&b"background_thread:true,dirty_decay_ms:1000,muzzy_decay_ms:0\0"[0]);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    everruns_agentd::cli::run(everruns_agentd_testkit::host()).await
}
