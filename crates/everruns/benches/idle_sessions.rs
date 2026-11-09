//! Idle-session memory: what does Engine keep for a session between turns?
//!
//! Data plane v2 (`knowledge/execution/data-plane-v2/`) gates plan step 1 on
//! memory per idle session: a cell's executor runs turns for many more
//! sessions than are active at once, so whatever Engine keeps for a session
//! after its turn ends is multiplied by every session the process has seen.
//!
//! `N` sessions each run one llmsim turn on [`MemoryHosting`] and their
//! handles are dropped. The heap is counted before, after, and after the
//! Engine is dropped, which splits it into:
//! - Engine-held bytes per idle session (gone with the Engine): the budget;
//! - authority-held bytes per session (the in-memory authority's log and
//!   records, which a cell keeps on disk and in S3, not in the executor).
//!
//! Decisions:
//! - A counting global allocator, not RSS: RSS keeps freed pages and moves in
//!   whole pages, so it can't resolve a few hundred bytes per session.
//! - `cargo test` runs this as a smoke (`test = true`): 1 024 sessions over
//!   a 32-session cache, checking the budget; `cargo bench` runs 20 000 over
//!   the default 1 024. Engine keeps up to the cache's capacity of idle
//!   sessions fully (about 5 KB each), so the per-session figure includes
//!   that share.
//!
//! Usage:
//!   cargo bench -p everruns --bench idle_sessions
//!   cargo bench -p everruns --bench idle_sessions -- --sessions 50000 --cache 4096

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::{Duration, Instant};

use everruns::hosting::{AgentBuild, MemoryHosting};
use everruns::{Agent, Engine, LlmSimConfig, Model, SessionId};

/// Engine-held bytes per idle session the gate allows (see the design's
/// "Step 1 measurements"). funout keeps ~20 KB of metadata per idle session
/// on a cell node; the executor side should be a small fraction of that.
const ENGINE_BYTES_PER_IDLE_SESSION: f64 = 2_048.0;

/// Sessions running their turn at once.
const WAVE: usize = 64;

struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

// SAFETY: forwards to the system allocator and only counts sizes.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            LIVE.fetch_add(
                new_size as isize - layout.size() as isize,
                Ordering::Relaxed,
            );
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn live() -> isize {
    LIVE.load(Ordering::Relaxed)
}

fn sessions_from_args() -> (bool, usize, usize) {
    let mut smoke = true;
    let mut sessions = None;
    let mut cache = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bench" => smoke = false,
            "--sessions" => sessions = args.next().and_then(|v| v.parse().ok()),
            "--cache" => cache = args.next().and_then(|v| v.parse().ok()),
            _ => {}
        }
    }
    (
        smoke,
        sessions.unwrap_or(if smoke { 1_024 } else { 20_000 }),
        cache.unwrap_or(if smoke { 32 } else { 1_024 }),
    )
}

/// Let background commits and released tasks finish before counting.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(300)).await;
}

async fn run_one(engine: &Engine, build: &AgentBuild) {
    let session = engine
        .create_registered(SessionId::new(), build)
        .await
        .expect("create session");
    let result = session.send_and_wait("hi").await.expect("turn");
    assert!(result.success, "{result:?}");
}

fn main() {
    let (smoke, sessions, cache) = sessions_from_args();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let build = AgentBuild::new("idle-sessions", "bench-v1");
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated_with_config(LlmSimConfig::lorem(20)))
        .build()
        .expect("valid agent");
    let hosting = MemoryHosting::default().without_prompts();
    let engine = Engine::builder()
        .hosting(hosting.clone())
        .register(build.clone(), agent)
        .session_cache_capacity(cache)
        .build();

    let (before, after, started) = runtime.block_on(async {
        // Warm up lazy statics and the registration's first build.
        run_one(&engine, &build).await;
        settle().await;
        let before = live();
        let started = Instant::now();
        let mut done = 0;
        while done < sessions {
            let wave = WAVE.min(sessions - done);
            let runs = (0..wave).map(|_| run_one(&engine, &build));
            futures::future::join_all(runs).await;
            done += wave;
        }
        settle().await;
        (before, live(), started)
    });
    let elapsed = started.elapsed();
    drop(engine);
    runtime.block_on(settle());
    let authority_only = live();

    let n = sessions as f64;
    let engine_held = (after - authority_only) as f64 / n;
    let authority_held = (authority_only - before) as f64 / n;
    println!(
        "idle_sessions ({}): {sessions} sessions, one llmsim turn each, handles dropped, session cache {cache}, {:.1} s",
        if smoke { "smoke" } else { "full" },
        elapsed.as_secs_f64()
    );
    println!("engine-held bytes per idle session:    {engine_held:>10.0}");
    println!("authority-held bytes per session:      {authority_held:>10.0}");
    assert!(
        engine_held <= ENGINE_BYTES_PER_IDLE_SESSION,
        "Engine keeps {engine_held:.0} bytes per idle session, budget {ENGINE_BYTES_PER_IDLE_SESSION}"
    );
}
