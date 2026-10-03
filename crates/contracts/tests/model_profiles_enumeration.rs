#![allow(clippy::unwrap_used)]

use everruns_contracts::model_profile_data::{
    all_profiles, profiles_for_provider, selected_profiles, selected_profiles_for_provider,
};

#[test]
fn selected_profiles_are_the_curated_registry() {
    assert!(!all_profiles().is_empty());
    assert_eq!(selected_profiles(), all_profiles());
    assert_eq!(
        selected_profiles_for_provider("openai"),
        profiles_for_provider("openai")
    );
    assert!(profiles_for_provider("unknown").is_empty());
    assert!(selected_profiles_for_provider("unknown").is_empty());
}

#[test]
fn entries_round_trip_without_conflating_families_or_aliases() {
    use everruns_contracts::model_profile_data::{
        ServiceKind, all_profile_entries, get_model_profile_by_key,
    };
    let entries = all_profile_entries();
    assert_eq!(
        all_profiles(),
        entries
            .iter()
            .map(|entry| entry.profile.clone())
            .collect::<Vec<_>>()
    );
    let mut keys = std::collections::HashSet::new();
    for entry in &entries {
        assert!(keys.insert(&entry.key), "duplicate {}", entry.key);
        assert_eq!(
            entry.key,
            format!("{}/{}", entry.vendor.slug(), entry.model_id)
        );
        assert_eq!(
            get_model_profile_by_key(&entry.key),
            Some(entry.profile.clone())
        );
    }
    let variant = entries
        .iter()
        .find(|entry| entry.model_id == "claude-opus-5-5[1m]")
        .unwrap();
    assert_ne!(variant.model_id, variant.profile.family);
    let nemotron = entries
        .iter()
        .find(|entry| entry.model_id == "nemotron-3-super-120b-a12b")
        .unwrap();
    assert_ne!(nemotron.model_id, nemotron.profile.family);
    assert!(
        nemotron
            .aliases
            .contains(&"nvidia/nemotron-3-super-120b-a12b".into())
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.service == ServiceKind::Embeddings)
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.service == ServiceKind::Realtime)
    );
}

#[test]
fn provider_entries_match_lookup_and_selection_is_a_subset() {
    use everruns_contracts::model_profile_data::{
        all_profile_entries, get_model_profile, get_model_profile_key, get_model_service_kind,
        get_model_vendor, profile_entries_for_provider,
    };
    let all = all_profile_entries();
    for provider in [
        "openai",
        "openrouter",
        "azure_openai",
        "openai_completions",
        "anthropic",
        "gemini",
        "meta",
        "mai",
        "llmsim",
        "bedrock",
        "unknown",
    ] {
        let entries = profile_entries_for_provider(provider);
        assert_eq!(
            profiles_for_provider(provider),
            entries
                .iter()
                .map(|entry| entry.profile.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            selected_profiles_for_provider(provider),
            profiles_for_provider(provider)
        );
        let expected: Vec<_> = all
            .iter()
            .filter(|entry| get_model_profile(provider, &entry.model_id).is_some())
            .map(|entry| &entry.key)
            .collect();
        assert_eq!(
            entries.iter().map(|entry| &entry.key).collect::<Vec<_>>(),
            expected
        );
        for entry in entries {
            for id in std::iter::once(&entry.model_id).chain(&entry.aliases) {
                assert_eq!(
                    get_model_profile(provider, id),
                    Some(entry.profile.clone()),
                    "{provider}/{id}"
                );
                assert_eq!(get_model_profile_key(provider, id), Some(entry.key.clone()));
                assert_eq!(get_model_vendor(provider, id), Some(entry.vendor));
                assert_eq!(get_model_service_kind(provider, id), entry.service);
            }
        }
    }
}

#[test]
fn provider_masks_and_curated_models_survive_enumeration() {
    use everruns_contracts::model_profile_data::profile_entries_for_provider;
    let openai = profile_entries_for_provider("openai");
    let astra = openai
        .iter()
        .find(|entry| entry.model_id == "gpt-6-astra")
        .unwrap();
    assert!(astra.profile.supports_phases);
    assert!(astra.profile.tool_search);
    assert!(astra.profile.speed.is_some());
    assert!(astra.profile.verbosity.is_some());
    for provider in ["openrouter", "azure_openai", "openai_completions"] {
        let entries = profile_entries_for_provider(provider);
        let astra = entries
            .iter()
            .find(|entry| entry.model_id == "gpt-6-astra")
            .unwrap();
        assert!(!astra.profile.supports_phases);
        assert!(!astra.profile.tool_search);
        assert!(astra.profile.speed.is_none());
        assert!(astra.profile.verbosity.is_none());
        assert!(!astra.profile.supports_server_compaction);
    }
    for (provider, required) in [
        ("openai", &["gpt-6-astra", "gpt-6-sol", "gpt-6-luna"][..]),
        ("anthropic", &["claude-opus-5-5", "claude-fable-5-1"][..]),
        (
            "meta",
            &["muse-spark-1.3", "muse-spark-1.3-contributor"][..],
        ),
        (
            "gemini",
            &["gemini-3.1-pro-preview", "gemini-3.5-flash"][..],
        ),
    ] {
        let entries = profile_entries_for_provider(provider);
        for id in required {
            assert!(
                entries.iter().any(|entry| entry.model_id == *id),
                "missing {provider}/{id}"
            );
        }
    }
    assert!(
        !profile_entries_for_provider("azure_openai")
            .iter()
            .any(|entry| entry.model_id.starts_with("muse-"))
    );
}

#[test]
fn returned_profiles_are_owned_and_order_is_repeatable() {
    let expected = all_profiles();
    let mut modified = all_profiles();
    modified[0].name.clear();
    assert_eq!(all_profiles(), expected);
    assert_ne!(modified, expected);
}
