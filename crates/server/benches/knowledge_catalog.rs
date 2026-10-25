//! Knowledge catalog pages on a million-file collection.
//!
//! The bench loads a synthetic collection of one million files into a private
//! embedded PostgreSQL database, then reads catalog pages at the start, middle
//! and end of it through `KnowledgeService::files_page`, the query behind
//! `GET /v1/knowledge/collections/{id}/files`.
//!
//! Decisions:
//! - A bench target (`harness = false`, not run by `cargo test`): loading a
//!   million rows takes about a minute, too long for every PR. The weekly
//!   `durable-bench.yml` runs it; clippy's `--all-targets` keeps it compiling,
//!   and `files_page_reads_the_path_index` checks the query plan on every run.
//! - It fails, not just reports, when a page is slow. A page served by the
//!   `(collection_id, path)` index takes a few milliseconds at any size; one
//!   that scans and sorts the collection takes hundreds. [`PAGE_BUDGET`] sits
//!   far from both, so machine noise does not trip it and a lost index does.
//! - The embedded database needs no `DATABASE_URL`, so it runs anywhere
//!   `cargo test -p everruns-server` does.
//!
//! Usage:
//!   cargo bench -p everruns-server --bench knowledge_catalog
//!   ... -- --files <n>    # catalog size, default 1,000,000

use std::sync::Arc;
use std::time::{Duration, Instant};

use everruns_contracts::knowledge::IndexMode;
use everruns_server::domains::knowledge::KnowledgeService;
use everruns_server::domains::knowledge::types::{CollectionInput, PageQuery};
use everruns_server::storage::StorageBackend;
use uuid::Uuid;

/// The slowest a page may be (median of [`SAMPLES`]) before the run fails.
const PAGE_BUDGET: Duration = Duration::from_millis(50);
const SAMPLES: usize = 5;
const PAGE_ROWS: u32 = 100;
/// A page's JSON stays bounded however large the collection is.
const PAGE_BYTES_MAX: usize = 200_000;

fn main() {
    let mut files: u32 = 1_000_000;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        // `cargo bench` passes `--bench`.
        if arg == "--files" {
            files = args
                .next()
                .and_then(|n| n.parse().ok())
                .expect("--files takes a number");
        }
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let slow = runtime.block_on(run(files));
    if !slow.is_empty() {
        eprintln!(
            "knowledge_catalog: pages over the {PAGE_BUDGET:?} budget: {}",
            slow.join(", ")
        );
        std::process::exit(1);
    }
}

async fn run(files: u32) -> Vec<String> {
    let service = KnowledgeService::files(Arc::new(StorageBackend::test_database()));
    service
        .db
        .replace_org_feature_flags(
            1,
            &std::collections::HashMap::from([("knowledge".into(), true)]),
        )
        .await
        .expect("enable knowledge");
    let created = service
        .create(
            1,
            CollectionInput {
                name: format!("catalog-bench-{}", Uuid::new_v4()),
                indexing: IndexMode::Off,
                ..Default::default()
            },
        )
        .await
        .expect("create collection");
    let collection = service
        .collection(1, created["id"].as_str().expect("collection id"))
        .await
        .expect("load collection");
    let pool = service.db.pool();
    let source: Uuid =
        sqlx::query_scalar("SELECT id FROM knowledge_sources WHERE collection_id=$1")
            .bind(collection.id)
            .fetch_one(pool)
            .await
            .expect("collection source");

    let started = Instant::now();
    sqlx::query("INSERT INTO knowledge_files(id,org_id,collection_id,source_id,path) SELECT gen_random_uuid(),$1,$2,$3,'/docs/'||lpad(n::text,7,'0')||'.md' FROM generate_series(1,$4) n")
        .bind(collection.org_id).bind(collection.id).bind(source).bind(files as i32)
        .execute(pool).await.expect("insert files");
    sqlx::query("INSERT INTO knowledge_versions(id,org_id,file_id,content_hash,size_bytes,mime_type,raw_key,raw_parts,metadata,extraction_status,source_uri) SELECT gen_random_uuid(),org_id,id,'hash',0,'text/markdown','knowledge/test/empty',1,'{}','ready','test://' FROM knowledge_files WHERE collection_id=$1")
        .bind(collection.id).execute(pool).await.expect("insert versions");
    sqlx::query("UPDATE knowledge_files f SET current_version=v.id FROM knowledge_versions v WHERE v.file_id=f.id AND f.collection_id=$1")
        .bind(collection.id).execute(pool).await.expect("link versions");
    sqlx::raw_sql("ANALYZE knowledge_files; ANALYZE knowledge_versions")
        .execute(pool)
        .await
        .expect("analyze");
    println!(
        "knowledge_catalog: {files} files loaded in {:?}",
        started.elapsed()
    );

    let middle = format!("/docs/{:07}.md", files / 2);
    let near_end = format!("/docs/{:07}.md", files.saturating_sub(PAGE_ROWS * 5));
    println!(
        "{:>20} {:>10} {:>10} {:>10}",
        "after", "median", "max", "bytes"
    );
    let mut slow = Vec::new();
    for after in ["", middle.as_str(), near_end.as_str()] {
        let mut times = Vec::with_capacity(SAMPLES);
        let mut bytes = 0;
        for _ in 0..SAMPLES {
            let started = Instant::now();
            let page = service
                .files_page(
                    &collection,
                    &PageQuery {
                        after: Some(after.into()),
                        limit: Some(PAGE_ROWS),
                        ..Default::default()
                    },
                )
                .await
                .expect("catalog page");
            times.push(started.elapsed());
            let rows = page["data"].as_array().expect("page rows").len();
            assert_eq!(rows, PAGE_ROWS as usize, "page after {after:?}");
            bytes = serde_json::to_vec(&page).expect("page json").len();
            assert!(
                bytes < PAGE_BYTES_MAX,
                "page after {after:?} is {bytes} bytes"
            );
        }
        times.sort();
        let median = times[SAMPLES / 2];
        println!(
            "{:>20} {:>10.2?} {:>10.2?} {:>10}",
            if after.is_empty() { "(start)" } else { after },
            median,
            times[SAMPLES - 1],
            bytes
        );
        if median > PAGE_BUDGET {
            slow.push(format!("after {after:?}: {median:?}"));
        }
    }
    slow
}
