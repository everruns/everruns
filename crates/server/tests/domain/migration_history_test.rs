// Lock migration filenames and SQL bodies that existing deployments have already
// recorded in _sqlx_migrations. Renaming or rewriting them breaks in-place
// startup with "previously applied but is missing/modified" errors.

use std::{fs, path::PathBuf};

fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

#[test]
fn eval_artifact_migrations_preserve_existing_database_history() {
    let dir = migrations_dir();
    let mut names = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();

    assert!(names.contains(&"016_eval_case_result_metadata.sql".to_string()));
    assert!(names.contains(&"017_eval_artifacts.sql".to_string()));
    assert!(
        !names.contains(&"016_v0.8.16.sql".to_string()),
        "rewriting 016/017 into a squashed release migration breaks deployments \
         that already applied the original files"
    );

    assert_eq!(
        fs::read_to_string(dir.join("016_eval_case_result_metadata.sql")).unwrap(),
        "ALTER TABLE eval_case_results\nADD COLUMN metadata JSONB;\n"
    );
    assert_eq!(
        fs::read_to_string(dir.join("017_eval_artifacts.sql")).unwrap(),
        "-- Add artifact specs to eval cases and collected artifact payloads to eval case results.\n\nALTER TABLE eval_cases\n    ADD COLUMN artifacts JSONB;\n\nALTER TABLE eval_case_results\n    ADD COLUMN artifacts JSONB;\n"
    );
}

#[test]
fn platform_chat_starter_migration_sanitizes_legacy_tags_before_backfill() {
    let migration =
        fs::read_to_string(migrations_dir().join("144_platform_chat_starter_unique.sql")).unwrap();
    let cleanup = migration
        .find("SET tags = array_remove(tags, 'platform-chat-starter')")
        .expect("migration must remove user-controlled legacy starter tags");
    let backfill = migration
        .find("WITH ranked AS")
        .expect("migration must elect a canonical starter");
    let unique_index = migration
        .find("CREATE UNIQUE INDEX idx_sessions_platform_chat_starter_owner")
        .expect("migration must enforce starter uniqueness");

    assert!(cleanup < backfill);
    assert!(backfill < unique_index);
}
