//! Every registered driver declares the environment variables its own vendor
//! SDK reads.
//!
//! The names belong to the drivers, not to a central derivation, so this is the
//! one place that sees all of them at once: it pins the published contract and
//! fails when a driver's declaration drifts from what is documented.
//!
//! Declaring a name is inert. Nothing here — and nothing on the server — reads
//! the process environment; only `EnvCredentialProvider`, constructed by
//! standalone/CLI/dev entrypoints, pairs a declaration with a real lookup.

use everruns_provider::provider::DriverId;
use everruns_worker::adapters::create_driver_registry;

/// The published table, in `docs/framework/credentials.md` order.
const DECLARED: &[(DriverId, &[&str])] = &[
    (DriverId::OpenAI, &["OPENAI_API_KEY", "OPENAI_BASE_URL"]),
    (
        DriverId::OpenAICompletions,
        &["OPENAI_API_KEY", "OPENAI_BASE_URL"],
    ),
    (
        DriverId::AzureOpenAI,
        &["AZURE_OPENAI_API_KEY", "AZURE_OPENAI_ENDPOINT"],
    ),
    (
        DriverId::Anthropic,
        &["ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL"],
    ),
    (DriverId::Gemini, &["GEMINI_API_KEY", "GOOGLE_API_KEY"]),
    (
        DriverId::OpenRouter,
        &["OPENROUTER_API_KEY", "OPENROUTER_BASE_URL"],
    ),
    (
        DriverId::Fireworks,
        &["FIREWORKS_API_KEY", "FIREWORKS_BASE_URL"],
    ),
    (
        DriverId::Meta,
        &[
            "LLAMA_API_KEY",
            "META_API_KEY",
            "MODEL_API_KEY",
            "LLAMA_BASE_URL",
        ],
    ),
    (
        DriverId::Bedrock,
        &[
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_REGION",
            "AWS_DEFAULT_REGION",
            "AWS_SESSION_TOKEN",
        ],
    ),
    (
        DriverId::Mai,
        &[
            "AZURE_AI_API_KEY",
            "AZURE_TENANT_ID",
            "AZURE_CLIENT_ID",
            "AZURE_CLIENT_SECRET",
            "AZURE_AI_ENDPOINT",
        ],
    ),
];

#[test]
fn every_driver_declares_its_vendors_variables() {
    let registry = create_driver_registry();
    for (id, expected) in DECLARED {
        let descriptor = registry
            .descriptor(id)
            .unwrap_or_else(|| panic!("{id} is registered"));
        assert_eq!(
            descriptor.declared_env_vars(),
            expected
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>(),
            "{id} declares different variables than the documented table"
        );
    }
}

#[test]
fn a_keyless_driver_declares_nothing() {
    let registry = create_driver_registry();
    let llmsim = registry
        .descriptor(&DriverId::LlmSim)
        .expect("llmsim is registered");
    assert!(
        llmsim.declared_env_vars().is_empty(),
        "the simulator needs no credentials, so it must not read any variable"
    );
}

#[test]
fn no_two_vendors_share_a_credential_variable() {
    // A shared name would let one vendor's key silently configure another
    // driver. `OPENAI_*` is the one deliberate exception: the Responses and
    // Chat Completions drivers are two protocols against one OpenAI account.
    let registry = create_driver_registry();
    let mut seen: Vec<(String, DriverId)> = Vec::new();
    for (id, _) in DECLARED {
        if id == &DriverId::OpenAICompletions {
            continue;
        }
        let descriptor = registry.descriptor(id).expect("registered");
        for name in descriptor.declared_env_vars() {
            if let Some((_, owner)) = seen.iter().find(|(seen, _)| seen == &name) {
                panic!("{name} is declared by both {owner} and {id}");
            }
            seen.push((name, id.clone()));
        }
    }
}
