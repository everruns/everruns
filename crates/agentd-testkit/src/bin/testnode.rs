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
    ballast();
    everruns_agentd::cli::run(everruns_agentd_testkit::host()).await
}

/// Memory a proof can make this process hold (`prove_memory_restart.py`).
///
/// Decision: the controlled restart at 98% of a node's limit (`pressure`,
/// level 4) is the last step before a kill, and real load does not reach it
/// on cue: the scaled soak peaks near 73%, since the shard sheds work first.
/// With `AGENTD_BALLAST_DIR` set, the process holds as many MiB as the file
/// named after its pid says, touched so they count in its resident set,
/// which is what a node with a configured limit reads. Test binary only.
fn ballast() {
    let Some(dir) = std::env::var_os("AGENTD_BALLAST_DIR") else {
        return;
    };
    let path = std::path::Path::new(&dir).join(std::process::id().to_string());
    std::thread::spawn(move || {
        let mut held: Vec<Vec<u8>> = Vec::new();
        loop {
            let want: usize = std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            while held.len() < want {
                held.push(vec![1u8; 1 << 20]);
            }
            held.truncate(want);
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
}
