use super::*;
#[tokio::test]
async fn knowledge_mount_bridge_is_read_only_and_supports_contextual_grep() {
    use crate::domains::knowledge::{KnowledgeService, types::*};
    use crate::storage::CreateSessionRow;
    use everruns_contracts::typed_id::PrincipalId;
    let adapters = test_adapters();
    let s = KnowledgeService::files(adapters.db.clone());
    let value = s
        .create(
            1,
            CollectionInput {
                name: "bridge".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let c = s
        .collection(1, value["id"].as_str().unwrap())
        .await
        .unwrap();
    let source: Source = sqlx::query_as("SELECT * FROM knowledge_sources WHERE collection_id=$1")
        .bind(c.id)
        .fetch_one(s.db.pool())
        .await
        .unwrap();
    s.ingest(
        &c,
        &source,
        FileInput {
            path: "/guide.md".into(),
            ..Default::default()
        },
        futures::stream::once(async {
            Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"before\nneedle\nafter"))
        }),
        "test://guide".into(),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let session =
        s.db.create_session(CreateSessionRow {
            org_id: 1,
            owner_principal_id: PrincipalId::from_seed(1),
            capabilities: serde_json::json!([{"ref":"knowledge","config":{"collections":[c.public_id()]}}]),
            ..Default::default()
        })
        .await
        .unwrap();
    let path = format!("/knowledge/{}/guide.md", c.public_id());
    assert!(
        adapters
            .read_file(1, session.id.uuid(), &path)
            .await
            .unwrap()
            .unwrap()
            .content
            .unwrap()
            .contains("needle")
    );
    assert!(
        adapters
            .write_file(1, session.id.uuid(), &path, "overwrite", "text")
            .await
            .is_err()
    );
    let result = adapters
        .grep_files_with_options(
            1,
            session.id.uuid(),
            "needle",
            &GrepOptions {
                path_pattern: Some(format!("/knowledge/{}", c.public_id())),
                before_context: 1,
                after_context: 1,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(result.total_matches, 1);
    assert_eq!(result.blocks[0].lines.len(), 3);
}
