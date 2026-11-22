//! `agentd`: the data plane v2 binary self-hosted installs and
//! production run. One binary, by subcommand: a shard `node`, an `executor`,
//! the `edge` router, and the restore tools (`cli.rs`). It serves no models
//! or tools of its own: an org's providers come from the control plane, as
//! sealed connections (`providers.rs`), and agents from their definitions.
//!
//! Decision: a separate binary from `agentd-testnode`, which adds simulated
//! models, test tools and a stub MCP egress for the test fleets and load
//! runs; none of that belongs in an install.

// Decision: jemalloc, as `agentd-testnode` measured (its doc has the numbers):
// glibc malloc held three times the Raft leader's live heap and never gave
// it back.
#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// jemalloc's options (a C string; `_RJEM_MALLOC_CONF` overrides them).
#[allow(non_upper_case_globals)]
#[unsafe(export_name = "_rjem_malloc_conf")]
pub static malloc_conf: Option<&u8> =
    Some(&b"background_thread:true,dirty_decay_ms:1000,muzzy_decay_ms:0\0"[0]);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    everruns_agentd::cli::run(everruns_agentd::StandardHost::new()).await
}
