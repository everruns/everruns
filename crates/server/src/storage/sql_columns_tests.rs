use super::models::SkillRow;
use everruns_server_macros::sql;

#[test]
fn columns_follow_field_order() {
    assert_eq!(
        SkillRow::COLUMNS,
        "id, public_id, org_id, name, description, license, compatibility, metadata, \
         allowed_tools, instructions, source_type, archive_data, status, version, created_at, \
         updated_at, archived_at, deleted_at"
    );
}

#[test]
fn sql_splices_columns_and_keeps_postgres_braces() {
    const QUERY: &str = sql!("SELECT {SkillRow} FROM skills WHERE metadata <> '{}'::jsonb");
    assert_eq!(
        QUERY,
        format!(
            "SELECT {} FROM skills WHERE metadata <> '{{}}'::jsonb",
            SkillRow::COLUMNS
        )
    );
}
