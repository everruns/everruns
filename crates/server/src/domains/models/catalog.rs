//! Persist model identity at catalog writes, independently from account credentials.
use everruns_contracts::model_profile_data::{self as profiles, ModelProfile, ServiceKind};
use serde_json::{Value, json};

pub const BINDING: &str = "everruns_catalog";

pub fn assign(
    driver: &str,
    model: &str,
    capabilities: &[String],
    profile_key: Option<&str>,
    service: Option<ServiceKind>,
    metadata: Option<Value>,
) -> anyhow::Result<Value> {
    let mut metadata = metadata.unwrap_or_else(|| json!({}));
    if !metadata.is_object() {
        anyhow::bail!(crate::errors::BadRequestError::new(
            "Model metadata must be an object"
        ));
    }
    // Extraction can use a model also registered for chat. Do not bind an
    // explicitly selected extraction endpoint to the chat profile.
    let inferred = profiles::get_model_profile_key(driver, model)
        .filter(|_| service != Some(ServiceKind::Extractions));
    let key = profile_key
        .map(str::to_owned)
        .or(inferred)
        .unwrap_or_else(|| format!("custom/{driver}/{model}"));
    let entry = profiles::all_profile_entries()
        .into_iter()
        .find(|entry| entry.key == key);
    if profile_key.is_some()
        && entry.is_none()
        && metadata[BINDING]["profile_key"].as_str() != Some(key.as_str())
    {
        anyhow::bail!(crate::errors::BadRequestError::new("Unknown model profile"));
    }
    if entry.is_some()
        && !profiles::profile_entries_for_provider(driver)
            .iter()
            .any(|entry| entry.key == key)
    {
        anyhow::bail!(crate::errors::BadRequestError::new(
            "Profile is not offered by this provider driver"
        ));
    }
    let selected = service.unwrap_or_else(|| {
        entry
            .as_ref()
            .map(|entry| entry.service)
            .unwrap_or_else(|| {
                capabilities
                    .iter()
                    .find_map(|tag| {
                        serde_json::from_value::<ServiceKind>(json!(tag.to_lowercase())).ok()
                    })
                    .unwrap_or(ServiceKind::Chat)
            })
    });
    if entry
        .as_ref()
        .is_some_and(|entry| entry.service != selected)
    {
        anyhow::bail!(crate::errors::BadRequestError::new(
            "Model service does not match its profile"
        ));
    }
    if entry.is_none() && metadata.get("discovered_profile").is_none() {
        metadata["discovered_profile"] = serde_json::to_value(ModelProfile::minimal(model))?;
    }
    metadata[BINDING] = json!({"profile_key": key, "service": selected});
    Ok(metadata)
}

pub fn service(metadata: Option<&Value>) -> ServiceKind {
    metadata
        .and_then(|metadata| serde_json::from_value(metadata[BINDING]["service"].clone()).ok())
        .unwrap_or_default()
}

pub fn key(metadata: Option<&Value>) -> String {
    metadata
        .and_then(|metadata| metadata[BINDING]["profile_key"].as_str())
        .unwrap_or_default()
        .to_owned()
}

/// The vendor whose own API a first-party driver is. Its models are called by
/// the bare id (`gpt-6.1-sol`), while gateways qualify it (`openai/gpt-6.1-sol`).
fn first_party_vendor(driver: &str) -> Option<&'static str> {
    match driver {
        "openai" | "azure_openai" => Some("openai"),
        "anthropic" => Some("anthropic"),
        "gemini" => Some("google"),
        "mistral" => Some("mistral"),
        "meta" => Some("meta"),
        "mai" => Some("microsoft"),
        _ => None,
    }
}

/// A model's provider-independent identity, `vendor/model`: the Model
/// Profile key for registry models, otherwise derived from how the driver
/// spells ids. `None` when the spelling says nothing about the vendor (a
/// custom endpoint's own names).
///
/// Decision: one normal form for "which model is this", so the same model
/// under two providers' spellings compares equal. Model sync matches what a
/// provider lists by it, and moving a model to another provider rewrites its
/// id through [`provider_spelling`].
pub fn identity(driver: &str, model_id: &str) -> Option<String> {
    if let Some(key) = profiles::get_model_profile_key(driver, model_id) {
        return Some(key);
    }
    let id = model_id.trim().to_ascii_lowercase();
    if let Some(vendor) = first_party_vendor(driver) {
        return (!id.is_empty() && !id.contains('/')).then(|| format!("{vendor}/{id}"));
    }
    if profiles::vendor_qualified_ids(driver) {
        let (vendor, model) = id.trim_start_matches('~').split_once('/')?;
        return (!vendor.is_empty() && !model.is_empty() && !model.contains('/'))
            .then(|| format!("{vendor}/{model}"));
    }
    None
}

/// A stored row's identity: its saved Model Profile binding when that names a
/// registry model, otherwise what its id says on `driver`.
pub fn row_identity(driver: &str, model_id: &str, metadata: Option<&Value>) -> Option<String> {
    let stored = key(metadata);
    if !stored.is_empty() && !stored.starts_with("custom/") {
        return Some(stored);
    }
    identity(driver, model_id)
}

/// How `driver` spells the model with this identity. `None` when it cannot
/// offer it: a first-party API serves only its own vendor's models.
pub fn provider_spelling(driver: &str, identity: &str) -> Option<String> {
    if let Some(id) = profiles::provider_model_id(driver, identity) {
        return Some(id.to_owned());
    }
    let (vendor, model) = identity.split_once('/')?;
    if let Some(own) = first_party_vendor(driver) {
        return (own == vendor).then(|| model.to_owned());
    }
    if profiles::vendor_qualified_ids(driver) {
        return Some(identity.to_owned());
    }
    None
}

pub fn profile(metadata: Option<&Value>) -> Option<ModelProfile> {
    let key = key(metadata);
    profiles::get_model_profile_by_key(&key).or_else(|| {
        metadata.and_then(|metadata| {
            serde_json::from_value(metadata["discovered_profile"].clone()).ok()
        })
    })
}

pub fn require_chat(metadata: Option<&Value>) -> anyhow::Result<()> {
    if service(metadata) != ServiceKind::Chat {
        anyhow::bail!(crate::errors::BadRequestError::new(
            "A chat model is required for this selection"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_model_has_one_identity_under_every_providers_spelling() {
        let sol = Some("openai/gpt-6.1-sol".to_string());
        assert_eq!(identity("openai", "gpt-6.1-sol"), sol);
        assert_eq!(identity("openrouter", "openai/gpt-6.1-sol"), sol);
        // Models the registry does not know normalize by the driver's rule.
        let x = Some("openai/gpt-x-preview".to_string());
        assert_eq!(identity("openai", "gpt-x-preview"), x);
        assert_eq!(identity("openrouter", "openai/gpt-x-preview"), x);
        assert_eq!(identity("openrouter", "OpenAI/GPT-X-Preview"), x);
        // A custom endpoint's own names carry no vendor.
        assert_eq!(identity("openai_completions", "my-finetune"), None);
        assert_eq!(identity("openai", "org/thing"), None);
    }

    #[test]
    fn a_model_is_spelled_the_way_each_provider_calls_it() {
        assert_eq!(
            provider_spelling("openrouter", "openai/gpt-6.1-sol").as_deref(),
            Some("openai/gpt-6.1-sol")
        );
        assert_eq!(
            provider_spelling("openai", "openai/gpt-6.1-sol").as_deref(),
            Some("gpt-6.1-sol")
        );
        assert_eq!(
            provider_spelling("openrouter", "openai/gpt-x-preview").as_deref(),
            Some("openai/gpt-x-preview")
        );
        // OpenAI's own API does not serve another vendor's models.
        assert_eq!(provider_spelling("openai", "anthropic/claude-x"), None);
        assert_eq!(
            provider_spelling("openai_completions", "openai/gpt-x"),
            None
        );
    }
    #[test]
    fn decisions_have_explicit_shared_identity_and_read_without_wire_guessing() {
        for (direct, gateway) in [
            ("jev-1.13.0", "typesafe/jev-1.13"),
            ("jev-1.14.0", "typesafe/jev-1.14"),
            ("jev-latest", "~typesafe/jev-latest"),
        ] {
            let a = assign("typesafe", direct, &[], None, None, None).unwrap();
            let b = assign("openrouter", gateway, &[], None, None, None).unwrap();
            assert_eq!(key(Some(&a)), key(Some(&b)));
            assert_eq!(service(Some(&b)), ServiceKind::Decisions);
            assert!(profile(Some(&b)).unwrap().decisions.is_some());
            assert!(
                assign(
                    "openrouter",
                    gateway,
                    &[],
                    None,
                    Some(ServiceKind::Chat),
                    None
                )
                .is_err()
            );
        }
        assert!(assign("openrouter", "x", &[], Some("unknown"), None, None).is_err());
    }
    #[test]
    fn explicit_extraction_does_not_inherit_a_chat_profile() {
        let metadata = assign(
            "openrouter",
            "openai/gpt-6.1-sol",
            &[],
            None,
            Some(ServiceKind::Extractions),
            None,
        )
        .unwrap();
        assert_eq!(service(Some(&metadata)), ServiceKind::Extractions);
        assert!(key(Some(&metadata)).starts_with("custom/"));
        // Other inferred catalog identities remain unchanged.
        let chat = assign("openrouter", "openai/gpt-6.1-sol", &[], None, None, None).unwrap();
        assert_eq!(service(Some(&chat)), ServiceKind::Chat);
    }
}
