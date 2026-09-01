//! A Mira `Subject` that drives an in-process Everruns Framework session — no
//! server, no HTTP, no database. Each sample gets a fresh [`Agent`] and
//! [`Session`] built from the matrix case:
//!
//! - target → Framework model id + provider (`anthropic`, `openai`, `openrouter`)
//! - `harness` axis → a [`HarnessProfile`](crate::profiles::HarnessProfile)
//!   (system prompt + capability set)
//! - `config` axis → a [`ConfigProfile`](crate::profiles::ConfigProfile)
//!   (iteration budget, parallel tool calls)
//! - `effort` axis → `Controls.reasoning.effort` on every input turn
//!
//! Sample `files` are seeded into an isolated temporary workspace before the
//! run; paths named by `metadata.expect_files` are read back into
//! `Transcript.files` afterwards so scorers can grade workspace state. Image
//! `attachments` are sent with the sample's first turn (vision cases).
//!
//! A case whose `metadata.requires` capabilities are not provided by the
//! active harness profile is **skipped**: the transcript carries a
//! `skipped` metadata key and every scorer returns N/A (see `scorers.rs`), so
//! crossing the full dataset with the `minimal` profile stays green.

use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use everruns::{
    Agent, CapabilityRef, ContentPart, Controls, EventStreamError, ImageContentPart,
    InMemoryEngine, InputMessage, Provider, ReasoningConfig, ReasoningEffort, Session,
    SessionEvent, SessionEventKind, WorkspacePolicy,
};

use mira::subject::summarize_events;
use mira::{ErrorKind, Part, RunCx, Sample, Source, Subject, Target, Transcript};

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
        // `default` means "send no override"; every other axis value must name a
        // real `ReasoningEffort`. Parsing against the enum rather than matching
        // the string list keeps the axis from drifting out of sync with it.
        let effort_override = if effort == DEFAULT_EFFORT {
            None
        } else {
            match ReasoningEffort::parse(effort) {
                Some(parsed) => Some(parsed),
                None => return Transcript::infra_error(format!("unknown effort '{effort}'")),
            }
        };

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

        // Non-text attachments (images) ride along with the first turn. An
        // attachment kind we can't map is a dataset-authoring fault.
        let attachments = match attachment_parts(sample) {
            Ok(parts) => parts,
            Err(e) => return Transcript::infra_error(e),
        };

        let handle = match build_session(sample, &cx.target, harness, config) {
            Ok(handle) => handle,
            // The runtime failed to build before the model ran — scaffolding,
            // so attribute to infra (scored N/A, retried).
            Err(e) => return Transcript::infra_error(format!("Framework build failed: {e}")),
        };
        let mut events = handle.session.events();

        for (index, turn) in sample.input.iter().enumerate() {
            let mut input = InputMessage::user(turn.clone());
            if index == 0 {
                input.content.extend(attachments.iter().cloned());
            }
            if let Some(effort) = effort_override {
                input.controls = Some(Controls {
                    reasoning: Some(ReasoningConfig {
                        effort: Some(effort),
                    }),
                    ..Default::default()
                });
            }
            let error = match handle.session.run(input).await {
                Ok(result) => {
                    transcript.final_response = result.response;
                    transcript.iterations += result.iterations;
                    if result.success {
                        None
                    } else {
                        Some(result.error.unwrap_or_else(|| "turn failed".into()))
                    }
                }
                Err(e) => Some(e.to_string()),
            };
            if let Some(msg) = error {
                // A route that cannot accept the sample's modality (e.g. no
                // image-input endpoint for this model) is a target/provider
                // gap, not a graded answer — skip the case (all scorers N/A),
                // mirroring how an unmet harness `requires` skips.
                if is_modality_unsupported(&msg) {
                    transcript.metadata.insert(
                        SKIPPED_KEY.into(),
                        format!("target route does not support this input modality: {msg}").into(),
                    );
                } else {
                    transcript.error_kind = classify_runtime_error(&msg);
                    transcript.error = Some(msg);
                }
                break;
            }
        }

        // Normalize the Framework event stream: usage + ordered tool-call names.
        loop {
            match events.try_recv() {
                Ok(Some(event)) => transcript.events.push(event_value(event)),
                Ok(None) => break,
                Err(error) => {
                    mark_event_stream_error(&mut transcript, error);
                    transcript.timing.duration_ms = started.elapsed().as_millis() as u64;
                    return transcript;
                }
            }
        }
        let (usage, _) = summarize_events(&transcript.events);
        transcript.usage = usage;
        // Tool names come from the Framework's `tool.completed` events —
        // mira's generic walk looks for `{name, input}` objects, which the
        // everruns event shape (`data.tool_name`) doesn't match.
        transcript.tool_calls = extract_tool_calls(&transcript.events);
        transcript.tool_calls_count = transcript.tool_calls.len();

        // Read back the workspace files the sample's expectations name, so
        // the `file_expectations` scorer grades real post-run state.
        for path in expected_file_paths(sample) {
            if let Some(content) = read_workspace_file(handle.workspace.path(), &path) {
                transcript.files.insert(path, content);
            }
        }

        transcript.timing.duration_ms = started.elapsed().as_millis() as u64;
        transcript
    }
}

fn mark_event_stream_error(transcript: &mut Transcript, error: EventStreamError) {
    transcript.error_kind = ErrorKind::Infra;
    transcript.error = Some(format!("Framework event stream incomplete: {error}"));
}

fn event_value(event: SessionEvent) -> serde_json::Value {
    let event_type = event.event_type().to_string();
    let event_id = event.event_id;
    let session_id = event.session_id;
    let turn_id = event.turn_id;
    let data = match event.kind {
        SessionEventKind::TurnStarted | SessionEventKind::TurnCompleted => serde_json::json!({}),
        SessionEventKind::TurnFailed { error } => serde_json::json!({ "error": error }),
        SessionEventKind::TurnCancelled => serde_json::json!({ "cancelled": true }),
        SessionEventKind::TextDelta { delta } => serde_json::json!({ "delta": delta }),
        SessionEventKind::ToolStarted {
            tool_call_id,
            tool_name,
        } => serde_json::json!({ "tool_call_id": tool_call_id, "tool_name": tool_name }),
        SessionEventKind::ToolCompleted {
            tool_call_id,
            tool_name,
            success,
        } => serde_json::json!({
            "tool_call_id": tool_call_id,
            "tool_name": tool_name,
            "success": success
        }),
        SessionEventKind::ToolProgress {
            tool_call_id,
            tool_name,
            message,
        } => serde_json::json!({
            "tool_call_id": tool_call_id,
            "tool_name": tool_name,
            "message": message
        }),
        SessionEventKind::Other { payload, .. } => payload,
        _ => serde_json::json!({}),
    };
    serde_json::json!({
        "type": event_type,
        "event_id": event_id,
        "session_id": session_id,
        "turn_id": turn_id,
        "data": data
    })
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

/// Tool names, in call order, from the Framework's `tool.completed` events.
fn extract_tool_calls(events: &[serde_json::Value]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("tool.completed"))
        .filter_map(|e| {
            e.get("data")
                .and_then(|d| d.get("tool_name"))
                .and_then(|n| n.as_str())
                .map(String::from)
        })
        .collect()
}

/// Map the sample's non-text attachments onto everruns content parts. Images
/// are supported (inline base64 or a URL / `data:` URI — the drivers decode
/// both); any other attachment kind is rejected.
fn attachment_parts(sample: &Sample) -> Result<Vec<ContentPart>, String> {
    sample
        .attachments
        .iter()
        .map(|part| match part {
            Part::Image { media_type, source } => Ok(ContentPart::Image(match source {
                Source::Data(b64) => ImageContentPart::from_base64(b64.clone(), media_type.clone()),
                Source::Uri(uri) => ImageContentPart::from_url(uri.clone()),
            })),
            other => Err(format!(
                "{}: unsupported attachment kind {other:?} (images only)",
                sample.id
            )),
        })
        .collect()
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

/// Read a text file from the Framework workspace.
fn read_workspace_file(root: &Path, path: &str) -> Option<String> {
    let root = std::fs::canonicalize(root).ok()?;
    let path = std::fs::canonicalize(workspace_path(&root, path).ok()?).ok()?;
    path.starts_with(&root)
        .then(|| std::fs::read_to_string(path).ok())
        .flatten()
}

struct EvalSession {
    session: Session,
    workspace: tempfile::TempDir,
}

/// Build a fresh Framework session for one matrix case. Fresh per sample so no
/// state leaks across cases.
fn build_session(
    sample: &Sample,
    target: &Target,
    harness: &'static HarnessProfile,
    config: &'static ConfigProfile,
) -> Result<EvalSession, String> {
    let workspace = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut builder = Agent::builder()
        .name(format!("generic-eval-{}", sample.id))
        .instructions(harness.system_prompt)
        .provider(provider(target)?)
        .model(target.model.clone())
        .max_iterations(config.max_iterations);

    for capability in harness.capabilities {
        builder = builder.capability(CapabilityRef::new(*capability));
    }
    if harness.capabilities.contains(&"session_file_system") {
        builder = builder
            .workspace(workspace.path())
            .workspace_policy(WorkspacePolicy::read_write());
    }
    if let Some(mode) = config.parallel_tool_calls_mode {
        builder = builder.capability(CapabilityRef::with_config(
            "parallel_tool_calls",
            serde_json::json!({ "mode": mode }),
        ));
    }

    for (path, content) in &sample.files {
        workspace_path(workspace.path(), path)?;
        builder = builder.file(path.clone(), content.clone());
    }

    let agent = builder.build().map_err(|error| error.to_string())?;
    Ok(EvalSession {
        session: InMemoryEngine::new().create(agent),
        workspace,
    })
}

fn workspace_path(root: &Path, model_path: &str) -> Result<PathBuf, String> {
    let relative = model_path
        .strip_prefix("/workspace/")
        .or_else(|| model_path.strip_prefix("workspace/"))
        .unwrap_or(model_path.trim_start_matches('/'));
    let path = Path::new(relative);
    if path.as_os_str().is_empty()
        || path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("invalid workspace path {model_path:?}"));
    }
    Ok(root.join(path))
}

/// Map the matrix target onto a Framework provider. Keys are read here (study
/// side); Mira targets stay key-free labels.
fn provider(target: &Target) -> Result<Provider, String> {
    let provider = match target.provider.as_str() {
        "anthropic" => {
            let key_name = "ANTHROPIC_API_KEY";
            let key = std::env::var(key_name).map_err(|_| format!("missing API key: {key_name}"))?;
            everruns_anthropic::provider("anthropic", key)
        }
        "openai" => {
            let key_name = "OPENAI_API_KEY";
            let key = std::env::var(key_name).map_err(|_| format!("missing API key: {key_name}"))?;
            everruns_openai::provider("openai", key)
        }
        "openrouter" => {
            let key_name = "OPENROUTER_API_KEY";
            let key = std::env::var(key_name).map_err(|_| format!("missing API key: {key_name}"))?;
            everruns_openrouter::provider("openrouter", key)
        }
        other => {
            return Err(format!(
                "unsupported provider '{other}' (supported: anthropic, openai, openrouter)"
            ));
        }
    };
    Ok(provider)
}

/// True when a provider error says the selected route cannot accept the
/// input modality at all (observed verbatim: OpenRouter's `404 {"message":
/// "No endpoints found that support image input"}` for models without a
/// vision-capable endpoint). Such cases skip rather than fail or retry.
fn is_modality_unsupported(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    ["image input", "audio input", "video input"]
        .iter()
        .any(|modality| m.contains(&format!("support {modality}")))
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
        "more credits",
        "credit balance",
        "payment required",
        "402",
        "insufficient funds",
        "insufficient_quota",
        "quota",
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
    fn event_stream_lag_is_an_infra_error() {
        let mut transcript = Transcript::default();

        mark_event_stream_error(&mut transcript, EventStreamError::Lagged { missed: 12 });

        assert!(transcript.errored_infra());
        assert_eq!(
            transcript.error.as_deref(),
            Some("Framework event stream incomplete: event stream lagged by 12 events")
        );
    }

    #[test]
    fn unsupported_provider_is_rejected() {
        let err = provider(&Target::new("x", "acme", "m")).unwrap_err();
        assert!(err.contains("unsupported provider"));
    }

    #[test]
    fn workspace_paths_cannot_escape_the_sample_root() {
        let root = Path::new("/tmp/eval-root");
        assert_eq!(
            workspace_path(root, "/workspace/report.md").unwrap(),
            root.join("report.md")
        );
        for path in ["../secret", "/workspace/../secret", "/", ""] {
            assert!(workspace_path(root, path).is_err(), "accepted {path:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn workspace_reads_reject_symlink_escapes() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(outside.path(), "secret").unwrap();
        symlink(outside.path(), workspace.path().join("result.txt")).unwrap();

        assert_eq!(
            read_workspace_file(workspace.path(), "/workspace/result.txt"),
            None
        );
    }

    #[test]
    fn billing_and_quota_errors_are_infra() {
        // Observed verbatim from providers mid-run; billing is never the
        // model's fault, so these must score N/A (and retry), not fail.
        for msg in [
            "OpenAI Responses API error (402 Payment Required): {\"error\":{\"message\":\"This request requires more credits, or fewer max_tokens.\"}}",
            "insufficient_quota: You exceeded your current quota, please check your plan and billing details.",
            "Anthropic API error (400 Bad Request): Your credit balance is too low to access the Anthropic API.",
        ] {
            assert_eq!(classify_runtime_error(msg), ErrorKind::Infra, "{msg}");
        }
        // A capability gap is not infra — it is handled separately as a skip
        // (see `is_modality_unsupported`), never retried.
        assert_eq!(
            classify_runtime_error("404 Not Found: No endpoints found that support image input"),
            ErrorKind::Subject
        );
    }

    #[test]
    fn modality_unsupported_routes_are_detected() {
        // Observed verbatim from OpenRouter for models with no vision route;
        // these skip the case (all scorers N/A) instead of failing it.
        assert!(is_modality_unsupported(
            "OpenAI Responses API error (404 Not Found): {\"error\":{\"message\":\"No endpoints found that support image input\",\"code\":404}}"
        ));
        assert!(!is_modality_unsupported("invalid image data"));
        assert!(!is_modality_unsupported("429 Too Many Requests"));
    }

    #[test]
    fn tool_calls_come_from_tool_completed_events() {
        // The everruns event shape (`data.tool_name`) — pinned so scorer
        // input doesn't silently go empty if the event schema drifts.
        let events = vec![
            serde_json::json!({"type": "turn.started", "data": {}}),
            serde_json::json!({"type": "tool.completed",
                "data": {"tool_call_id": "c1", "tool_name": "write_file",
                         "success": true, "status": "success"}}),
            serde_json::json!({"type": "tool.completed",
                "data": {"tool_call_id": "c2", "tool_name": "read_file",
                         "success": true, "status": "success"}}),
            serde_json::json!({"type": "turn.completed", "data": {}}),
        ];
        assert_eq!(extract_tool_calls(&events), vec!["write_file", "read_file"]);
    }
}
