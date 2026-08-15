use std::sync::Arc;

use crate::annotation_hook::{AnnotationProvider, VerifierProvider};
use crate::capabilities::CapabilityRegistry;
use crate::output_guardrail::{OutputGuardrail, PostGenerationProvider};

pub(super) struct OutputHooks {
    pub(super) streaming: Vec<(String, serde_json::Value, Arc<dyn OutputGuardrail>)>,
    pub(super) post_generation: Vec<PostGenerationProvider>,
    pub(super) annotations: Vec<AnnotationProvider>,
    pub(super) citation_verifiers: Vec<VerifierProvider>,
}

pub(super) fn collect_output_hooks(
    registry: &CapabilityRegistry,
    configs: &[crate::CapabilityRef],
) -> OutputHooks {
    let streaming = configs
        .iter()
        .filter_map(|config| {
            let capability_id = config.capability_id();
            let capability = registry.get(capability_id)?;
            let guardrails = capability.output_guardrails();
            (!guardrails.is_empty()).then(|| {
                guardrails
                    .into_iter()
                    .map(|guardrail| {
                        (
                            capability_id.to_string(),
                            config.config_value().clone(),
                            guardrail,
                        )
                    })
                    .collect::<Vec<_>>()
            })
        })
        .flatten()
        .collect();

    let post_generation = configs
        .iter()
        .filter_map(|config| {
            let capability_id = config.capability_id();
            let capability = registry.get(capability_id)?;
            let providers = capability.post_output_guardrails_with_config(config.config_value());
            (!providers.is_empty()).then(|| {
                providers
                    .into_iter()
                    .map(|provider| PostGenerationProvider {
                        capability_id: capability_id.to_string(),
                        provider,
                    })
                    .collect::<Vec<_>>()
            })
        })
        .flatten()
        .collect();

    let annotations = configs
        .iter()
        .filter_map(|config| {
            let capability_id = config.capability_id();
            let capability = registry.get(capability_id)?;
            let providers =
                capability.post_output_annotation_hooks_with_config(config.config_value());
            (!providers.is_empty()).then(|| {
                providers
                    .into_iter()
                    .map(|provider| AnnotationProvider {
                        capability_id: capability_id.to_string(),
                        provider,
                    })
                    .collect::<Vec<_>>()
            })
        })
        .flatten()
        .collect();

    let citation_verifiers = configs
        .iter()
        .filter_map(|config| {
            let capability_id = config.capability_id();
            let capability = registry.get(capability_id)?;
            let provider = capability.citation_verifier_with_config(config.config_value())?;
            Some(VerifierProvider {
                capability_id: capability_id.to_string(),
                provider,
            })
        })
        .collect();

    OutputHooks {
        streaming,
        post_generation,
        annotations,
        citation_verifiers,
    }
}
