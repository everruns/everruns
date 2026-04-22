use crate::api::users::User;
use crate::storage::UserRow;

pub fn row_to_user(row: UserRow) -> User {
    let roles: Vec<String> = serde_json::from_value(row.roles).unwrap_or_default();
    User {
        id: row.id.to_string(),
        email: row.email,
        name: row.name,
        avatar_url: row.avatar_url,
        roles,
        auth_provider: row.auth_provider,
        created_at: row.created_at,
    }
}
