// Model String Resolver
//
// This module provides resolution for "provider/model" format model identifiers.
// It supports:
// - Parsing "provider/model" format (e.g., "anthropic/opus-4.5", "openai/gpt-5.2")
// - Resolving model aliases to actual model IDs (e.g., "opus-4.5" → "claude-opus-4-5-20251101")
// - Falling back to the original string if no alias matches
//
// Anthropic model aliases are resolved to the latest dated version.
// OpenAI model names are passed through as-is (OpenAI handles aliases server-side).

use crate::llm_models::LlmProviderType;

/// Parsed model string containing provider and model information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedModelString {
    /// Provider type (openai, anthropic, etc.)
    pub provider: Option<LlmProviderType>,
    /// Model identifier (may be alias or full model ID)
    pub model: String,
    /// Resolved model ID (alias resolved to actual model ID)
    pub resolved_model: String,
}

/// Parse and resolve a model string in "provider/model" format.
///
/// # Examples
///
/// ```
/// use everruns_core::model_string_resolver::parse_model_string;
///
/// // With provider prefix
/// let parsed = parse_model_string("anthropic/opus-4.5");
/// assert_eq!(parsed.provider, Some(everruns_core::llm_models::LlmProviderType::Anthropic));
/// assert_eq!(parsed.model, "opus-4.5");
/// assert_eq!(parsed.resolved_model, "claude-opus-4-5-20251101");
///
/// // Without provider prefix (just model ID)
/// let parsed = parse_model_string("gpt-5.2");
/// assert_eq!(parsed.provider, None);
/// assert_eq!(parsed.model, "gpt-5.2");
/// assert_eq!(parsed.resolved_model, "gpt-5.2");
/// ```
pub fn parse_model_string(model_string: &str) -> ParsedModelString {
    // Check for "provider/model" format
    if let Some((provider_str, model)) = model_string.split_once('/') {
        let provider = parse_provider(provider_str);
        let resolved_model = resolve_model_alias(provider.as_ref(), model);

        ParsedModelString {
            provider,
            model: model.to_string(),
            resolved_model,
        }
    } else {
        // No provider prefix, try to infer from model name
        let provider = infer_provider_from_model(model_string);
        let resolved_model = resolve_model_alias(provider.as_ref(), model_string);

        ParsedModelString {
            provider,
            model: model_string.to_string(),
            resolved_model,
        }
    }
}

/// Parse a provider string to LlmProviderType
fn parse_provider(provider_str: &str) -> Option<LlmProviderType> {
    match provider_str.to_lowercase().as_str() {
        "openai" => Some(LlmProviderType::Openai),
        "openai_completions" | "openai-completions" => Some(LlmProviderType::OpenaiCompletions),
        "anthropic" => Some(LlmProviderType::Anthropic),
        "gemini" | "google" => Some(LlmProviderType::Gemini),
        "llmsim" => Some(LlmProviderType::LlmSim),
        _ => None,
    }
}

/// Infer provider from model name patterns
fn infer_provider_from_model(model: &str) -> Option<LlmProviderType> {
    let lower = model.to_lowercase();

    // OpenAI patterns
    if lower.starts_with("gpt-")
        || lower.starts_with("o1")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
    {
        return Some(LlmProviderType::Openai);
    }

    // Anthropic patterns
    if lower.starts_with("claude-")
        || lower.contains("opus")
        || lower.contains("sonnet")
        || lower.contains("haiku")
    {
        return Some(LlmProviderType::Anthropic);
    }

    // Gemini patterns
    if lower.starts_with("gemini-") || lower.starts_with("gemini_") {
        return Some(LlmProviderType::Gemini);
    }

    // LlmSim patterns
    if lower.starts_with("llmsim") {
        return Some(LlmProviderType::LlmSim);
    }

    None
}

/// Resolve model aliases to actual model IDs
///
/// This is primarily used for Anthropic models where we want to support
/// shorthand aliases like "opus-4.5" that resolve to "claude-opus-4-5-20251101".
///
/// OpenAI handles aliases server-side, so we pass those through as-is.
fn resolve_model_alias(provider: Option<&LlmProviderType>, model: &str) -> String {
    match provider {
        Some(LlmProviderType::Anthropic) => resolve_anthropic_alias(model),
        Some(LlmProviderType::Openai | LlmProviderType::OpenaiCompletions) => {
            resolve_openai_alias(model)
        }
        Some(LlmProviderType::Gemini) => resolve_gemini_alias(model),
        Some(LlmProviderType::LlmSim) => resolve_llmsim_alias(model),
        None => {
            // Try to infer and resolve
            if let Some(inferred) = infer_provider_from_model(model) {
                resolve_model_alias(Some(&inferred), model)
            } else {
                model.to_string()
            }
        }
    }
}

/// Resolve Anthropic model aliases to dated versions
///
/// Alias format examples:
/// - "opus-4.5" → "claude-opus-4-5-20251101"
/// - "sonnet-4.5" → "claude-sonnet-4-5-20250929"
/// - "haiku-4.5" → "claude-haiku-4-5-20251001"
/// - "sonnet-4" → "claude-sonnet-4-20250514"
/// - "opus-4" → "claude-opus-4-20250514"
/// - "sonnet-3.7" → "claude-3-7-sonnet-20250219"
/// - "sonnet-3.5" → "claude-3-5-sonnet-20241022"
///
/// Full model IDs are passed through unchanged.
fn resolve_anthropic_alias(model: &str) -> String {
    let lower = model.to_lowercase();

    // If it already starts with "claude-", assume it's a full model ID
    if lower.starts_with("claude-") {
        return model.to_string();
    }

    // Anthropic aliases - resolve to latest dated versions
    // Format: "{tier}-{version}" → "claude-{tier}-{version}-{date}"
    match lower.as_str() {
        // Claude 4.5 series (newest)
        "opus-4.5" | "opus-4-5" | "opus4.5" => "claude-opus-4-5-20251101".to_string(),
        "sonnet-4.5" | "sonnet-4-5" | "sonnet4.5" => "claude-sonnet-4-5-20250929".to_string(),
        "haiku-4.5" | "haiku-4-5" | "haiku4.5" => "claude-haiku-4-5-20251001".to_string(),

        // Claude 4 series
        "opus-4" | "opus4" => "claude-opus-4-20250514".to_string(),
        "sonnet-4" | "sonnet4" => "claude-sonnet-4-20250514".to_string(),

        // Claude 3.7 series
        "sonnet-3.7" | "sonnet-3-7" | "sonnet3.7" => "claude-3-7-sonnet-20250219".to_string(),

        // Claude 3.5 series
        "sonnet-3.5" | "sonnet-3-5" | "sonnet3.5" => "claude-3-5-sonnet-20241022".to_string(),
        "haiku-3.5" | "haiku-3-5" | "haiku3.5" => "claude-3-5-haiku-20241022".to_string(),

        // Claude 3 series (legacy)
        "opus-3" | "opus3" => "claude-3-opus-20240229".to_string(),
        "sonnet-3" | "sonnet3" => "claude-3-sonnet-20240229".to_string(),
        "haiku-3" | "haiku3" => "claude-3-haiku-20240307".to_string(),

        // Shorthand for latest of each tier (resolve to 4.5 series)
        "opus" => "claude-opus-4-5-20251101".to_string(),
        "sonnet" => "claude-sonnet-4-5-20250929".to_string(),
        "haiku" => "claude-haiku-4-5-20251001".to_string(),

        // Not an alias, return as-is
        _ => model.to_string(),
    }
}

/// Resolve OpenAI model aliases
///
/// OpenAI mostly handles aliases server-side, but we can add convenience aliases.
fn resolve_openai_alias(model: &str) -> String {
    let lower = model.to_lowercase();

    match lower.as_str() {
        // Latest flagship model shortcuts
        "latest" | "flagship" => "gpt-5.2".to_string(),
        "codex" => "gpt-5.2-codex".to_string(),
        "pro" => "gpt-5.2-pro".to_string(),

        // Otherwise pass through as-is
        _ => model.to_string(),
    }
}

/// Resolve Google Gemini model aliases
///
/// Gemini model names are mostly passed through as-is since they use
/// a simple naming scheme (gemini-2.5-pro, gemini-2.5-flash, etc.).
fn resolve_gemini_alias(model: &str) -> String {
    let lower = model.to_lowercase();

    match lower.as_str() {
        // Convenience aliases for latest models
        "pro" | "latest" => "gemini-2.5-pro".to_string(),
        "flash" => "gemini-2.5-flash".to_string(),

        // Otherwise pass through as-is
        _ => model.to_string(),
    }
}

/// Resolve LlmSim model aliases
fn resolve_llmsim_alias(model: &str) -> String {
    let lower = model.to_lowercase();

    match lower.as_str() {
        "default" | "sim" => "llmsim-default".to_string(),
        _ => model.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_anthropic_with_prefix() {
        let parsed = parse_model_string("anthropic/opus-4.5");
        assert_eq!(parsed.provider, Some(LlmProviderType::Anthropic));
        assert_eq!(parsed.model, "opus-4.5");
        assert_eq!(parsed.resolved_model, "claude-opus-4-5-20251101");
    }

    #[test]
    fn test_parse_anthropic_sonnet() {
        let parsed = parse_model_string("anthropic/sonnet-4.5");
        assert_eq!(parsed.provider, Some(LlmProviderType::Anthropic));
        assert_eq!(parsed.resolved_model, "claude-sonnet-4-5-20250929");
    }

    #[test]
    fn test_parse_anthropic_haiku() {
        let parsed = parse_model_string("anthropic/haiku-4.5");
        assert_eq!(parsed.resolved_model, "claude-haiku-4-5-20251001");
    }

    #[test]
    fn test_parse_anthropic_shorthand() {
        // Just "opus" should resolve to latest (4.5)
        let parsed = parse_model_string("anthropic/opus");
        assert_eq!(parsed.resolved_model, "claude-opus-4-5-20251101");

        let parsed = parse_model_string("anthropic/sonnet");
        assert_eq!(parsed.resolved_model, "claude-sonnet-4-5-20250929");
    }

    #[test]
    fn test_parse_anthropic_full_model_id() {
        // Full model IDs should pass through unchanged
        let parsed = parse_model_string("anthropic/claude-opus-4-5-20251101");
        assert_eq!(parsed.resolved_model, "claude-opus-4-5-20251101");
    }

    #[test]
    fn test_parse_openai_with_prefix() {
        let parsed = parse_model_string("openai/gpt-5.2");
        assert_eq!(parsed.provider, Some(LlmProviderType::Openai));
        assert_eq!(parsed.model, "gpt-5.2");
        assert_eq!(parsed.resolved_model, "gpt-5.2");
    }

    #[test]
    fn test_parse_openai_aliases() {
        let parsed = parse_model_string("openai/latest");
        assert_eq!(parsed.resolved_model, "gpt-5.2");

        let parsed = parse_model_string("openai/codex");
        assert_eq!(parsed.resolved_model, "gpt-5.2-codex");
    }

    #[test]
    fn test_parse_without_prefix_infers_openai() {
        let parsed = parse_model_string("gpt-5.2");
        assert_eq!(parsed.provider, Some(LlmProviderType::Openai));
        assert_eq!(parsed.resolved_model, "gpt-5.2");
    }

    #[test]
    fn test_parse_without_prefix_infers_anthropic() {
        let parsed = parse_model_string("claude-opus-4-5-20251101");
        assert_eq!(parsed.provider, Some(LlmProviderType::Anthropic));
        assert_eq!(parsed.resolved_model, "claude-opus-4-5-20251101");
    }

    #[test]
    fn test_parse_anthropic_version_formats() {
        // All these should resolve to the same model
        let formats = ["opus-4.5", "opus-4-5", "opus4.5"];
        for format in formats {
            let parsed = parse_model_string(&format!("anthropic/{}", format));
            assert_eq!(
                parsed.resolved_model, "claude-opus-4-5-20251101",
                "Failed for format: {}",
                format
            );
        }
    }

    #[test]
    fn test_parse_anthropic_3x_series() {
        let parsed = parse_model_string("anthropic/sonnet-3.7");
        assert_eq!(parsed.resolved_model, "claude-3-7-sonnet-20250219");

        let parsed = parse_model_string("anthropic/sonnet-3.5");
        assert_eq!(parsed.resolved_model, "claude-3-5-sonnet-20241022");

        let parsed = parse_model_string("anthropic/haiku-3.5");
        assert_eq!(parsed.resolved_model, "claude-3-5-haiku-20241022");
    }

    #[test]
    fn test_parse_anthropic_claude4_series() {
        let parsed = parse_model_string("anthropic/sonnet-4");
        assert_eq!(parsed.resolved_model, "claude-sonnet-4-20250514");

        let parsed = parse_model_string("anthropic/opus-4");
        assert_eq!(parsed.resolved_model, "claude-opus-4-20250514");
    }

    #[test]
    fn test_parse_llmsim() {
        let parsed = parse_model_string("llmsim/default");
        assert_eq!(parsed.provider, Some(LlmProviderType::LlmSim));
        assert_eq!(parsed.resolved_model, "llmsim-default");
    }

    #[test]
    fn test_parse_unknown_model() {
        let parsed = parse_model_string("unknown-provider/unknown-model");
        assert_eq!(parsed.provider, None);
        assert_eq!(parsed.model, "unknown-model");
        assert_eq!(parsed.resolved_model, "unknown-model");
    }

    #[test]
    fn test_infer_provider_from_model_name() {
        // GPT models
        assert_eq!(
            infer_provider_from_model("gpt-4o"),
            Some(LlmProviderType::Openai)
        );
        assert_eq!(
            infer_provider_from_model("o1"),
            Some(LlmProviderType::Openai)
        );
        assert_eq!(
            infer_provider_from_model("o3-mini"),
            Some(LlmProviderType::Openai)
        );

        // Claude models
        assert_eq!(
            infer_provider_from_model("claude-3-5-sonnet"),
            Some(LlmProviderType::Anthropic)
        );
        assert_eq!(
            infer_provider_from_model("opus-4.5"),
            Some(LlmProviderType::Anthropic)
        );

        // Unknown
        assert_eq!(infer_provider_from_model("unknown-model"), None);
    }
}
