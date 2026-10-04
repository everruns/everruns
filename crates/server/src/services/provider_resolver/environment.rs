/// Get default API key from environment variable based on provider type.
///
/// Environment variables (for development convenience):
/// - DEFAULT_OPENAI_API_KEY: Fallback API key for OpenAI providers
/// - DEFAULT_ANTHROPIC_API_KEY: Fallback API key for Anthropic providers
///
/// These are only used when the provider doesn't have an API key set in the database.
pub fn get_default_api_key_from_env(provider_type: &str) -> Option<String> {
    get_default_api_key_with_lookup(provider_type, |name| std::env::var(name).ok())
}

/// Testable version with injectable env lookup.
pub(super) fn get_default_api_key_with_lookup<F>(
    provider_type: &str,
    env_lookup: F,
) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    match provider_type.to_lowercase().as_str() {
        "openai" => env_lookup("DEFAULT_OPENAI_API_KEY").filter(|s| !s.is_empty()),
        "openrouter" => env_lookup("DEFAULT_OPENROUTER_API_KEY").filter(|s| !s.is_empty()),
        "azure_openai" => env_lookup("DEFAULT_AZURE_OPENAI_API_KEY").filter(|s| !s.is_empty()),
        "anthropic" => env_lookup("DEFAULT_ANTHROPIC_API_KEY").filter(|s| !s.is_empty()),
        "gemini" => env_lookup("DEFAULT_GEMINI_API_KEY").filter(|s| !s.is_empty()),
        "fireworks" => env_lookup("DEFAULT_FIREWORKS_API_KEY").filter(|s| !s.is_empty()),
        "meta" => env_lookup("DEFAULT_META_API_KEY").filter(|s| !s.is_empty()),
        "bedrock" => {
            // Construct JSON credentials from Bedrock-specific or generic AWS env vars.
            let access_key_id = env_lookup("AWS_BEDROCK_ACCESS_KEY_ID")
                .or_else(|| env_lookup("AWS_ACCESS_KEY_ID"))
                .filter(|s| !s.is_empty())?;
            let secret_access_key = env_lookup("AWS_BEDROCK_SECRET_ACCESS_KEY")
                .or_else(|| env_lookup("AWS_SECRET_ACCESS_KEY"))
                .filter(|s| !s.is_empty())?;
            let region = env_lookup("AWS_BEDROCK_REGION")
                .or_else(|| env_lookup("AWS_REGION"))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "us-east-1".to_string());
            let session_token = env_lookup("AWS_BEDROCK_SESSION_TOKEN")
                .or_else(|| env_lookup("AWS_SESSION_TOKEN"))
                .filter(|s| !s.is_empty());

            let mut cred = serde_json::json!({
                "access_key_id": access_key_id,
                "secret_access_key": secret_access_key,
                "region": region,
            });
            if let Some(token) = session_token {
                cred["session_token"] = serde_json::Value::String(token);
            }
            Some(cred.to_string())
        }
        _ => None,
    }
}
