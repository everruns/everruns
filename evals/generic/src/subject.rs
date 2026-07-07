//! A Mira `Subject` that drives the **local** `everruns-runtime` in-process —
//! no server, no HTTP, no database. Each sample gets a fresh
//! `InProcessRuntime` built from the matrix case:
//!
//! - target → provider driver + `ResolvedModel` (`anthropic`, `openai`)
//! - `harness` axis → a [`HarnessProfile`](crate::profiles::HarnessProfile)
//!   (system prompt + capability set)
//! - `config` axis → a [`ConfigProfile`](crate::profiles::ConfigProfile)
//!   (iteration budget, parallel tool calls)
//! - `effort` axis → `Controls.reasoning.effort` on every input turn
//!
//! Sample `files` are seeded into the in-memory session workspace before the
//! run; paths named by `metadata.expect_files` are read back into
//! `Transcript.files` afterwards so scorers can grade workspace state.
//!
//! A case whose `metadata.requires` capabilities are not provided by the
//! active harness profile is **skipped**: the transcript carries a
//! `skipped` metadata key and every scorer returns N/A (see `scorers.rs`), so
//! crossing the full dataset with the `minimal` profile stays green.

use std::time::Instant;

use everruns_core::capabilities::AgentCapabilityConfig;
use everruns_core::driver_registry::DriverRegistry;
use everruns_core::typed_id::SessionId;
use everruns_core::{
    CapabilityRegistry, Controls, DriverId, InputMessage, PlatformDefinition, ReasoningConfig,
    ResolvedModel,
};
use everruns_runtime::{InProcessRuntime, InProcessRuntimeBuilder};

use mira::subject::summarize_events;
use mira::{ErrorKind, RunCx, Sample, Subject, Target, Transcript};

use crate::profiles::{
    self, ConfigProfile, DEFAULT_CONFIG, DEFAULT_EFFORT, DEFAULT_HARNESS, HarnessProfile,
};

/// Transcript metadata key marking a case the subject did not run because the
/// harness profile lacks a required capability. Scorers N/A on it.
pub const SKIPPED_KEY: &str = "skipped";

pub struct GenericRuntimeSubject;

#[async_trait::async_trait]
impl Subject for GenericRuntimeSubject {
    async fn run(&self, sample: &Sample, cx: &RunCx) -> Transcript {
        let started = Instant::now();

        let harness_name = cx.param("harness").unwrap_or(DEFAULT_HARNESS);
        let config_name = cx.param("config").unwrap_or(DEFAULT_CONFIG);
        let effort = cx.param("effort").unwrap_or(DEFAULT_EFFORT);

        // Axis values come from env/CLI: a typo is a configuration fault, not
        // a model failure — surface as infra so the case reads N/A + message.
        let Some(harness) = profiles::harness_profile(harness_name) else {
            return Transcript::infra_error(format!("unknown harness profile '{harness_name}'"));
        };
        let Some(config) = profiles::config_profile(config_name) else {
            return Transcript::infra_error(format!("unknown config profile '{config_name}'"));
        };
        if !profiles::EFFORT_VALUES.contains(&effort) {
            return Transcript::infra_error(format!("unknown effort '{effort}'"));
        }

        let mut transcript = Transcript::default();
        transcript
            .metadata
            .insert("harness".into(), harness_name.into());
        transcript
            .metadata
            .insert("config".into(), config_name.into());
        transcript.metadata.insert("effort".into(), effort.into());

        // Incompatible (sample × harness) combo → skip, don't fail.
        if let Some(missing) = missing_capability(sample, harness) {
            transcript.metadata.insert(
                SKIPPED_KEY.into(),
                format!("harness '{harness_name}' lacks required capability '{missing}'").into(),
            );
            return transcript;
        }

        let (runtime, session_id) = match build_runtime(sample, &cx.target, harness, config).await {
            Ok(handle) => handle,
            // The runtime failed to build before the model ran — scaffolding,
            // so attribute to infra (scored N/A, retried).
            Err(e) => return Transcript::infra_error(format!("runtime build failed: {e}")),
        };

        for turn in &sample.input {
            let mut input = InputMessage::user(turn.clone());
            if effort != DEFAULT_EFFORT {
                input.controls = Some(Controls {
                    reasoning: Some(ReasoningConfig {
                        effort: Some(effort.to_string()),
                    }),
                    ..Default::default()
                });
            }
            match runtime.run_turn(session_id, input).await {
                Ok(result) => {
                    transcript.final_response = result.response;
                    transcript.iterations += result.iterations;
                    if !result.success {
                        let msg = result.error.unwrap_or_else(|| "turn failed".into());
                        transcript.error_kind = classify_runtime_error(&msg);
                        transcript.error = Some(msg);
                        break;
                    }
                }
                Err(e) => {
                    let msg = e.to_string();
                    transcript.error_kind = classify_runtime_error(&msg);
                    transcript.error = Some(msg);
                    break;
                }
            }
        }

        // Normalize the event stream: usage + ordered tool-call names.
        if let Ok(events) = runtime.events().await {
            for event in &events {
                if let Ok(value) = serde_json::to_value(event) {
                    transcript.events.push(value);
                }
            }
        }
        let (usage, tools) = summarize_events(&transcript.events);
        transcript.usage = usage;
        transcript.tool_calls = tools;
        transcript.tool_calls_count = transcript.tool_calls.len();

        // Read back the workspace files the sample's expectations name, so
        // the `file_expectations` scorer grades real post-run state.
        for path in expected_file_paths(sample) {
            if let Some(content) = read_workspace_file(&runtime, session_id, &path).await {
                transcript.files.insert(path, content);
            }
        }

        transcript.timing.duration_ms = started.elapsed().as_millis() as u64;
        transcript
    }
}

/// First capability in `metadata.requires` the harness profile does not
/// provide, if any.
fn missing_capability(sample: &Sample, harness: &HarnessProfile) -> Option<String> {
    let requires = sample.metadata.get("requires")?.as_array()?;
    requires
        .iter()
        .filter_map(|v| v.as_str())
        .find(|cap| !harness.capabilities.contains(cap))
        .map(String::from)
}

/// Paths named by `metadata.expect_files[].path`.
fn expected_file_paths(sample: &Sample) -> Vec<String> {
    sample
        .metadata
        .get("expect_files")
        .and_then(|v| v.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| e.get("path").and_then(|p| p.as_str()))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Read a text file from the in-memory session workspace, tolerating the
/// leading-`/` normalization the file store applies to paths.
async fn read_workspace_file(
    runtime: &InProcessRuntime,
    session_id: SessionId,
    path: &str,
) -> Option<String> {
    for candidate in [
        path.to_string(),
        format!("/{}", path.trim_start_matches('/')),
    ] {
        if let Ok(Some(file)) = runtime.read_file(session_id, &candidate).await
            && let Some(content) = file.content
        {
            return Some(content);
        }
    }
    None
}

/// Build a fresh in-process runtime for one matrix case. Fresh per sample so
/// no state leaks across cases.
async fn build_runtime(
    sample: &Sample,
    target: &Target,
    harness: &'static HarnessProfile,
    config: &'static ConfigProfile,
) -> Result<(InProcessRuntime, SessionId), String> {
    let mut drivers = DriverRegistry::new();
    everruns_anthropic::register_driver(&mut drivers);
    everruns_openai::register_driver(&mut drivers);
    let platform = PlatformDefinition::new(CapabilityRegistry::with_builtins(), drivers);

    let session_id = SessionId::new();
    let sample_id = sample.id.clone();
    let mut builder = InProcessRuntimeBuilder::new()
        .platform_definition(platform)
        .default_model(resolved_model(target)?)
        .single_session(move |s| {
            let mut s = s
                .session_id(session_id)
                .harness(harness.name, harness.system_prompt)
                .agent("generic-eval-agent", "")
                .agent_max_iterations(config.max_iterations)
                .session_title(format!("generic eval: {sample_id}"))
                .tag("generic-evals");
            for cap in harness.capabilities {
                s = s.with_capability(*cap);
            }
            if let Some(mode) = config.parallel_tool_calls_mode {
                s = s.with_capability(AgentCapabilityConfig::with_config(
                    "parallel_tool_calls",
                    serde_json::json!({ "mode": mode }),
                ));
            }
            s
        });

    for (path, content) in &sample.files {
        builder = builder.seed_text_file(session_id, path.clone(), content.clone());
    }

    let runtime = builder.build().await.map_err(|e| e.to_string())?;
    Ok((runtime, session_id))
}

/// Map the matrix target onto an everruns `ResolvedModel`. Keys are read here
/// (study side); Mira targets stay key-free labels.
fn resolved_model(target: &Target) -> Result<ResolvedModel, String> {
    let (provider_type, api_key) = match target.provider.as_str() {
        "anthropic" => (DriverId::Anthropic, std::env::var("ANTHROPIC_API_KEY").ok()),
        "openai" => (DriverId::OpenAI, std::env::var("OPENAI_API_KEY").ok()),
        other => {
            return Err(format!(
                "unsupported provider '{other}' (supported: anthropic, openai)"
            ));
        }
    };
    Ok(ResolvedModel {
        model: target.model.clone(),
        provider_type,
        api_key,
        base_url: None,
        provider_metadata: None,
    })
}

/// Classify a runtime/provider error string: transient scaffolding faults are
/// infra (scored N/A, retried); everything else is a real, scoreable subject
/// failure so a genuine model error is never silently excused.
fn classify_runtime_error(message: &str) -> ErrorKind {
    if mira::is_rate_limited(message) {
        return ErrorKind::Infra;
    }
    let m = message.to_ascii_lowercase();
    const INFRA_SIGNALS: &[&str] = &[
        "budget",
        "billing",
        "out of credit",
        "insufficient funds",
        "service unavailable",
        "503",
        "502",
        "500",
        "bad gateway",
        "gateway timeout",
        "timed out",
        "timeout",
        "connection refused",
        "connection reset",
        "connection closed",
        "broken pipe",
        "network unreachable",
        "network error",
        "dns error",
        "tls handshake",
        "temporarily unavailable",
        "missing api key",
        "no api key",
    ];
    if INFRA_SIGNALS.iter().any(|s| m.contains(s)) {
        ErrorKind::Infra
    } else {
        ErrorKind::Subject
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mira::RunCx;

    // Both the skip and axis-validation paths return before any provider is
    // contacted, so tests can use a real-shaped target without a key.
    fn cx() -> RunCx {
        RunCx::new(Target::new("anthropic/test", "anthropic", "test-model"))
    }

    #[tokio::test]
    async fn unmet_requires_skips_the_case() {
        let subject = GenericRuntimeSubject;
        let sample = Sample::new("needs-fs", "list files")
            .meta("requires", serde_json::json!(["session_file_system"]));
        let mut cx = cx();
        cx.params.insert("harness".into(), "minimal".into());
        let t = subject.run(&sample, &cx).await;
        assert!(
            t.metadata.contains_key(SKIPPED_KEY),
            "expected a skip marker"
        );
        assert!(t.error.is_none());
    }

    #[tokio::test]
    async fn unknown_axis_values_are_infra() {
        let subject = GenericRuntimeSubject;
        let sample = Sample::new("a", "hi");
        for (axis, value) in [("harness", "nope"), ("config", "nope"), ("effort", "nope")] {
            let mut cx = cx();
            cx.params.insert(axis.into(), value.into());
            let t = subject.run(&sample, &cx).await;
            assert!(t.errored_infra(), "{axis}={value} should be infra");
        }
    }

    #[test]
    fn unsupported_provider_is_rejected() {
        let err = resolved_model(&Target::new("x", "acme", "m")).unwrap_err();
        assert!(err.contains("unsupported provider"));
    }
}
