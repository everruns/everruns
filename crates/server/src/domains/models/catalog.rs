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
    let inferred = profiles::get_model_profile_key(driver, model);
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
    fn decisions_have_explicit_shared_identity_and_read_without_wire_guessing() {
        let a = assign("typesafe", "jev-1.13.0", &[], None, None, None).unwrap();
        let b = assign("openrouter", "typesafe/jev-1.13", &[], None, None, None).unwrap();
        assert_eq!(key(Some(&a)), key(Some(&b)));
        assert_eq!(service(Some(&b)), ServiceKind::Decisions);
        assert!(profile(Some(&b)).unwrap().decisions.is_some());
        assert!(
            assign(
                "openrouter",
                "typesafe/jev-1.13",
                &[],
                None,
                Some(ServiceKind::Chat),
                None
            )
            .is_err()
        );
        assert!(assign("openrouter", "x", &[], Some("unknown"), None, None).is_err());
    }
}
