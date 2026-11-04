//! One-level folder listing (`tree=true`): the loose index scan behind the
//! Knowledge file browser.

use super::tests::{create, fixture, write};
use super::*;
use everruns_contracts::knowledge::IndexMode;
use serde_json::Value;
use types::*;

#[tokio::test]
async fn folder_page_lists_immediate_children() {
    let s = fixture();
    let c = create(&s, IndexMode::Off).await;
    for p in [
        "/index.md",
        "/advanced/budgets.md",
        "/advanced/deep/nested.md",
        "/advanced-old/legacy.md",
        "/api/a.md",
        "/api/b.md",
        "/zeta.md",
    ] {
        write(&s, &c, p, b"text").await;
    }
    let level = |path: &'static str, after: Option<String>, limit: u32| {
        let s = &s;
        let c = &c;
        async move {
            s.files_page(
                c,
                &PageQuery {
                    path: Some(path.into()),
                    after,
                    limit: Some(limit),
                    tree: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
        }
    };
    let entries = |page: &Value| -> Vec<(String, String)> {
        page["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                (
                    e["kind"].as_str().unwrap().to_string(),
                    e["path"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    };
    let kinds = |items: &[(&str, &str)]| -> Vec<(String, String)> {
        items
            .iter()
            .map(|(k, p)| (k.to_string(), p.to_string()))
            .collect()
    };

    // Entries come in path byte order: `-` sorts before `/`, so the
    // `/advanced-old` folder precedes `/advanced`.
    let root = level("/", None, 100).await;
    assert_eq!(
        entries(&root),
        kinds(&[
            ("folder", "/advanced-old"),
            ("folder", "/advanced"),
            ("folder", "/api"),
            ("file", "/index.md"),
            ("file", "/zeta.md"),
        ])
    );
    assert!(root["next_cursor"].is_null());
    assert_eq!(root["data"][3]["version"]["mime_type"], "text/markdown");

    // `/advanced` is the folder, never its `/advanced-old` sibling.
    let advanced = level("/advanced/", None, 100).await;
    assert_eq!(
        entries(&advanced),
        kinds(&[
            ("file", "/advanced/budgets.md"),
            ("folder", "/advanced/deep"),
        ])
    );
    assert_eq!(advanced["data"][1]["name"], "deep");

    // Paging resumes after a folder without revisiting its files.
    let first = level("/", None, 2).await;
    assert_eq!(
        entries(&first),
        kinds(&[("folder", "/advanced-old"), ("folder", "/advanced")])
    );
    let cursor = first["next_cursor"].as_str().unwrap().to_string();
    let second = level("/", Some(cursor), 2).await;
    assert_eq!(
        entries(&second),
        kinds(&[("folder", "/api"), ("file", "/index.md")])
    );
    let cursor = second["next_cursor"].as_str().unwrap().to_string();
    let third = level("/", Some(cursor), 2).await;
    assert_eq!(entries(&third), kinds(&[("file", "/zeta.md")]));
    assert!(third["next_cursor"].is_null());

    assert!(
        level("/missing/", None, 100).await["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

/// A folder costs one index probe, not a read of every file under it, so
/// listing the root of a large collection stays cheap.
#[tokio::test]
async fn folder_page_skips_whole_subfolders() {
    let s = fixture();
    let c = create(&s, IndexMode::Off).await;
    let source: Uuid =
        sqlx::query_scalar("SELECT id FROM knowledge_sources WHERE collection_id=$1")
            .bind(c.id)
            .fetch_one(s.db.pool())
            .await
            .unwrap();
    sqlx::query("INSERT INTO knowledge_files(id,org_id,collection_id,source_id,path) SELECT gen_random_uuid(),$1,$2,$3,'/f'||(n%3)||'/'||lpad(n::text,7,'0')||'.md' FROM generate_series(1,20000) n").bind(c.org_id).bind(c.id).bind(source).execute(s.db.pool()).await.unwrap();
    sqlx::raw_sql("ANALYZE knowledge_files")
        .execute(s.db.pool())
        .await
        .unwrap();
    let explain = format!(
        "EXPLAIN (ANALYZE, FORMAT JSON) {}",
        repository::FOLDER_PAGE_SQL
    );
    let plan: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(explain))
        .bind(c.org_id)
        .bind(c.id)
        .bind("")
        .bind("/")
        .bind(101_i64)
        .fetch_one(s.db.pool())
        .await
        .unwrap();
    fn rows_read(plan: &Value) -> f64 {
        let own = if plan["Relation Name"] == "knowledge_files" {
            plan["Actual Rows"].as_f64().unwrap_or_default()
                * plan["Actual Loops"].as_f64().unwrap_or(1.0)
        } else {
            0.0
        };
        own + plan["Plans"]
            .as_array()
            .into_iter()
            .flatten()
            .map(rows_read)
            .sum::<f64>()
    }
    let read = rows_read(&plan[0]["Plan"]);
    assert!(read < 100.0, "read {read} file rows: {plan:#}");
    let page = s
        .files_page(
            &c,
            &PageQuery {
                tree: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page["data"].as_array().unwrap().len(), 3);
}
