use serde_json::Value;

#[derive(Debug, Clone)]
pub struct CreateOrganizationConnectionRow {
    pub org_id: i64,
    pub name: String,
    pub provider: String,
    pub access_token_encrypted: Vec<u8>,
    pub provider_username: Option<String>,
    pub provider_metadata: Option<Value>,
}
