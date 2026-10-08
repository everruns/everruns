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

#[test]
fn sql_prefixes_columns_with_a_table_alias() {
    let query: &'static str = sql!("SELECT {SkillRow as s}, o.name FROM skills s JOIN orgs o");
    assert!(query.starts_with("SELECT s.id, s.public_id, s.org_id, s.name, "));
    assert!(query.ends_with("s.archived_at, s.deleted_at, o.name FROM skills s JOIN orgs o"));
    let prefixed: Vec<String> = SkillRow::COLUMNS
        .split(", ")
        .map(|column| format!("s.{column}"))
        .collect();
    assert_eq!(
        query,
        format!(
            "SELECT {}, o.name FROM skills s JOIN orgs o",
            prefixed.join(", ")
        )
    );
}
