use everruns_provider::{
    DriverId, ServiceKind, all_profile_entries, all_profiles, profile_entries_for_provider,
    profiles_for_provider, selected_profiles, selected_profiles_for_provider,
};

#[test]
fn public_reexports_support_an_offline_chat_menu() {
    assert_eq!(all_profiles(), everruns_model_profiles::all_profiles());
    assert_eq!(
        selected_profiles(),
        everruns_model_profiles::selected_profiles()
    );
    assert_eq!(
        all_profile_entries(),
        everruns_model_profiles::all_profile_entries()
    );
    for provider in [
        DriverId::OpenAI,
        DriverId::Anthropic,
        DriverId::OpenRouter,
        DriverId::Meta,
        DriverId::external("unknown"),
    ] {
        assert_eq!(
            profiles_for_provider(&provider),
            everruns_model_profiles::profiles_for_provider(provider.as_str())
        );
        assert_eq!(
            selected_profiles_for_provider(&provider),
            profiles_for_provider(&provider)
        );
        let entries = profile_entries_for_provider(&provider);
        assert_eq!(
            entries,
            everruns_model_profiles::profile_entries_for_provider(provider.as_str())
        );
        let menu: Vec<_> = entries
            .iter()
            .filter(|entry| entry.service == ServiceKind::Chat)
            .map(|entry| entry.model_id.as_str())
            .collect();
        if provider == DriverId::OpenAI {
            assert!(menu.contains(&"gpt-6-astra"));
            assert!(!menu.contains(&"gpt-realtime-2"));
            assert!(!menu.contains(&"text-embedding-3-small"));
        }
    }
}
