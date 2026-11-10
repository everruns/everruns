//! A cell node binary with simulated models and test tools (tests only).
//! Agents come from the clients.

// Decision: a sharded allocator, not the system one. Under glibc malloc the
// Raft leader (many threads writing segments, snapshots and turns at once)
// held three times its live heap: 1.3 GB of arenas for 420 MB in use after
// 40k turns, which malloc_trim does not return. funout measured that with
// jemalloc; this repository's binaries use mimalloc for the same reason.
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    everruns_cell::cli::run(everruns_cell_testkit::host()).await
}
