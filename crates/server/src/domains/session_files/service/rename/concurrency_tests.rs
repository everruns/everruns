use super::{super::*, tests::seed};
use std::time::Duration;

#[tokio::test]
async fn directory_rename_rolls_back_when_a_waiting_child_becomes_readonly() {
    let db = Arc::new(StorageBackend::test_database());
    let session = db.create_test_session().await;
    seed(&db, session, "/source", true, false).await;
    let file = seed(&db, session, "/source/file", false, false).await;
    let mut writer = db.tx_pool().begin().await.unwrap();
    sqlx::query("SELECT id FROM workspace_files WHERE id = $1 FOR UPDATE")
        .bind(file.id)
        .fetch_one(&mut *writer)
        .await
        .unwrap();

    let moving_db = db.clone();
    let moving = tokio::spawn(async move {
        moving_db
            .move_session_file(session.uuid(), "/source", "/destination")
            .await
    });
    // Wait for the move's UPDATE to reach the held row, after its preflight
    // check. This exercises PostgreSQL's real READ COMMITTED interleaving.
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let blocked: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity
                 WHERE datname = current_database() AND pid <> pg_backend_pid()
                   AND wait_event_type = 'Lock' AND query LIKE '%UPDATE workspace_files%')",
            )
            .fetch_one(db.pool())
            .await
            .unwrap();
            if blocked {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("move did not reach the held child row");
    sqlx::query("UPDATE workspace_files SET is_readonly = TRUE WHERE id = $1")
        .bind(file.id)
        .execute(&mut *writer)
        .await
        .unwrap();
    writer.commit().await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(30), moving)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err(), "a readonly child must roll back the move");
    assert!(
        db.get_session_file_info(session.uuid(), "/source")
            .await
            .unwrap()
            .is_some()
    );
    let unchanged = db
        .get_session_file_info(session.uuid(), "/source/file")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.id, file.id);
    assert!(unchanged.is_readonly);
    assert!(
        db.get_session_file_info(session.uuid(), "/destination")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.get_session_file_info(session.uuid(), "/destination/file")
            .await
            .unwrap()
            .is_none()
    );
}
