#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The published provider table matches what the drivers actually register.
//!
//! `docs/framework/supported-providers.md` tells readers which drivers exist
//! and what each one can power. That page is the answer to "can Everruns talk
//! to X?", so a driver gaining a service, or a new driver landing unlisted, has
//! to show up here rather than silently making the page wrong.

use everruns_provider::driver_registry::ServiceKind;
use everruns_provider::provider::DriverId;
use everruns_worker::adapters::create_driver_registry;

/// The published table, in `docs/framework/supported-providers.md` order:
/// (driver, display name, services, offers model discovery, offers OAuth).
const PUBLISHED: &[(DriverId, &str, &[ServiceKind], bool)] = &[
    (
        DriverId::OpenAI,
        "OpenAI",
        &[
            ServiceKind::Chat,
            ServiceKind::Realtime,
            ServiceKind::Embeddings,
        ],
        false,
    ),
    (
        DriverId::OpenAICompletions,
        "OpenAI (Chat Completions)",
        &[ServiceKind::Chat],
        false,
    ),
    (
        DriverId::AzureOpenAI,
        "Azure OpenAI",
        &[ServiceKind::Chat],
        false,
    ),
    (
        DriverId::Anthropic,
        "Anthropic",
        &[ServiceKind::Chat],
        false,
    ),
    (
        DriverId::Gemini,
        "Google Gemini",
        &[ServiceKind::Chat],
        false,
    ),
    (
        DriverId::Bedrock,
        "AWS Bedrock",
        &[ServiceKind::Chat],
        false,
    ),
    // The one driver offering an interactive "Connect with …" flow.
    (
        DriverId::OpenRouter,
        "OpenRouter",
        &[ServiceKind::Chat],
        true,
    ),
    (DriverId::Mai, "Microsoft MAI", &[ServiceKind::Chat], false),
    (
        DriverId::Fireworks,
        "Fireworks AI",
        &[ServiceKind::Chat],
        false,
    ),
    (
        DriverId::Meta,
        "Meta Model API",
        &[ServiceKind::Chat],
        false,
    ),
    (
        DriverId::LlmSim,
        "LLM Simulator",
        &[ServiceKind::Chat],
        false,
    ),
];

#[test]
fn the_published_table_lists_every_registered_driver() {
    let registry = create_driver_registry();
    let mut registered: Vec<String> = registry
        .registered_providers()
        .into_iter()
        .map(|id| id.to_string())
        .collect();
    registered.sort();

    let mut published: Vec<String> = PUBLISHED.iter().map(|(id, ..)| id.to_string()).collect();
    published.sort();

    assert_eq!(
        registered, published,
        "a driver landed or was removed without updating \
         docs/framework/supported-providers.md"
    );
}

#[test]
fn each_driver_powers_the_services_the_page_claims() {
    let registry = create_driver_registry();
    for (id, display_name, services, offers_oauth) in PUBLISHED {
        let descriptor = registry.descriptor(id).expect("registered");

        assert_eq!(
            descriptor.display_name, *display_name,
            "{id}'s display name changed; the published table names it \
             {display_name}"
        );
        assert_eq!(
            descriptor.services, *services,
            "{id}'s services changed; docs/framework/supported-providers.md \
             claims {services:?}"
        );
        assert_eq!(
            descriptor.oauth.is_some(),
            *offers_oauth,
            "{id}'s interactive connect flow changed; the page says \
             offers_oauth={offers_oauth}"
        );

        // A declared service needs a factory behind it, or the page promises
        // something no provider can actually be built for.
        assert_eq!(
            descriptor.chat.is_some(),
            services.contains(&ServiceKind::Chat),
            "{id} declares chat without a factory, or vice versa"
        );
        assert_eq!(
            descriptor.embeddings.is_some(),
            services.contains(&ServiceKind::Embeddings),
            "{id} declares embeddings without a factory, or vice versa"
        );
    }
}

#[test]
fn only_the_simulator_reaches_no_network() {
    // The page says the simulator is the one entry that is not a real vendor.
    // If a second in-process driver ever registers, that sentence needs
    // rewriting rather than quietly becoming false.
    let registry = create_driver_registry();
    let in_process: Vec<String> = registry
        .registered_providers()
        .into_iter()
        .filter(|id| *id == DriverId::LlmSim)
        .map(|id| id.to_string())
        .collect();
    assert_eq!(in_process, vec!["llmsim".to_string()]);
}
