#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The upstream AG-UI 1.0 fixture corpus, run against these types.
//!
//! For every `valid/*.json` fixture: the type parses it, re-serializes
//! without dropping anything, and the output validates against the schema.
//! For every `invalid/*.json` fixture: the schema rejects it, and the type
//! rejects it too unless the violation is one the types deliberately
//! tolerate on input (see `tolerated`).

use std::fs;
use std::path::{Path, PathBuf};

use everruns_ag_ui::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

fn spec_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("spec/1.0")
}

fn validator(def: &str) -> jsonschema::Validator {
    let mut schema: Value = serde_json::from_str(SCHEMA_JSON).unwrap();
    schema["$ref"] = json!(format!("#/$defs/{def}"));
    jsonschema::validator_for(&schema).unwrap()
}

fn assert_schema_valid(def: &str, value: &Value) {
    let validator = validator(def);
    let errors: Vec<String> = validator
        .iter_errors(value)
        .map(|e| format!("{} at {}", e, e.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "{def} rejected {value}:\n{}",
        errors.join("\n")
    );
}

/// Every key of `original` is in `reserialized` with an equal value
/// (recursively), so a round trip dropped nothing. Defaults the types write
/// (`role`, a tool call's `type`) may be added.
fn assert_superset(original: &Value, reserialized: &Value, path: &str) {
    match (original, reserialized) {
        (Value::Object(a), Value::Object(b)) => {
            for (key, value) in a {
                let Some(other) = b.get(key) else {
                    // A whole-field null reads as absent (absent means absent).
                    assert!(value.is_null(), "round trip dropped {path}/{key}");
                    continue;
                };
                assert_superset(value, other, &format!("{path}/{key}"));
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "array length changed at {path}");
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                assert_superset(x, y, &format!("{path}/{i}"));
            }
        }
        _ => assert_eq!(original, reserialized, "value changed at {path}"),
    }
}

fn fixtures(def: &str, kind: &str) -> Vec<(PathBuf, Value)> {
    let dir = spec_dir().join("fixtures").join(def).join(kind);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "json")
                && !p.to_string_lossy().ends_with(".expect.json")
        })
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let value = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
            (p, value)
        })
        .collect()
}

/// Violations the types accept on input on purpose. Tolerance is the 1.0
/// consumer rule for these: a whole-field `null` reads as absent (every
/// optional field is an `Option`, so this is uniform rather than limited to
/// the fields 0.x historically sent as `null`),
/// unknown members are stripped rather than fatal, and numeric bounds,
/// patterns and `minItems` are a producer's obligation, not a parse error.
fn tolerated(path: &Path, value: &Value) -> bool {
    let expect: Value =
        serde_json::from_str(&fs::read_to_string(path.with_extension("expect.json")).unwrap())
            .unwrap();
    let keyword = expect["keyword"].as_str().unwrap_or_default();
    let instance = value
        .pointer(expect["instanceLocation"].as_str().unwrap_or_default())
        .unwrap_or(&Value::Null);
    instance.is_null()
        || matches!(
            keyword,
            "unevaluatedProperties" | "minimum" | "maximum" | "pattern" | "minItems"
        )
}

fn check<T: DeserializeOwned + Serialize>(def: &str) -> usize {
    let mut seen = 0;
    for (path, value) in fixtures(def, "valid") {
        // Event fixtures parse through the `Event` union, which owns `type`.
        let parsed: T = serde_json::from_value(value.clone())
            .unwrap_or_else(|e| panic!("{} did not parse: {e}", path.display()));
        let out = serde_json::to_value(&parsed).unwrap();
        // Fixtures named `*-ignored-*` carry members the protocol tells
        // readers to ignore, so dropping them is the point.
        if !path.to_string_lossy().contains("-ignored-") {
            assert_superset(&value, &out, &path.display().to_string());
        }
        assert_schema_valid(def, &out);
        let again: T = serde_json::from_value(out.clone()).unwrap();
        assert_eq!(serde_json::to_value(&again).unwrap(), out, "not idempotent");
        seen += 1;
    }
    let validator = validator(def);
    for (path, value) in fixtures(def, "invalid") {
        assert!(
            !validator.is_valid(&value),
            "schema accepted invalid fixture {}",
            path.display()
        );
        if !tolerated(&path, &value) {
            assert!(
                serde_json::from_value::<T>(value).is_err(),
                "type accepted invalid fixture {}",
                path.display()
            );
        }
        seen += 1;
    }
    seen
}

#[test]
fn upstream_fixture_corpus() {
    macro_rules! defs {
        ($($def:literal => $ty:ty),* $(,)?) => {{
            let mut covered = Vec::new();
            $(
                assert!(check::<$ty>($def) > 0, "no fixtures for {}", $def);
                covered.push($def);
            )*
            covered
        }};
    }
    let covered = defs! {
        "ActivityDeltaEvent" => Event,
        "ActivitySnapshotEvent" => Event,
        "AgentCapabilities" => AgentCapabilities,
        "CustomEvent" => Event,
        "ExecutionCapabilities" => ExecutionCapabilities,
        "FileSource" => PartSource,
        "Interrupt" => Interrupt,
        "MessagesSnapshotEvent" => Event,
        "MultiAgentCapabilities" => MultiAgentCapabilities,
        "RawEvent" => Event,
        "ReasoningEncryptedValueEvent" => Event,
        "ReasoningEndEvent" => Event,
        "ReasoningMessageChunkEvent" => Event,
        "ReasoningMessageContentEvent" => Event,
        "ReasoningMessageEndEvent" => Event,
        "ReasoningMessageStartEvent" => Event,
        "ReasoningStartEvent" => Event,
        "ResumeEntry" => ResumeEntry,
        "RunAgentInput" => RunAgentInput,
        "RunErrorEvent" => Event,
        "RunFinishedEvent" => Event,
        "RunStartedEvent" => Event,
        "StateDeltaEvent" => Event,
        "StateSnapshotEvent" => Event,
        "StepFinishedEvent" => Event,
        "StepStartedEvent" => Event,
        "SubagentErrorEvent" => Event,
        "SubagentFinishedEvent" => Event,
        "SubagentInfo" => SubagentInfo,
        "SubagentStartedEvent" => Event,
        "TextMessageChunkEvent" => Event,
        "TextMessageContentEvent" => Event,
        "TextMessageEndEvent" => Event,
        "TextMessageStartEvent" => Event,
        "Tool" => Tool,
        "ToolCallArgsEvent" => Event,
        "ToolCallChunkEvent" => Event,
        "ToolCallEndEvent" => Event,
        "ToolCallResultEvent" => Event,
        "ToolCallStartEvent" => Event,
        "ToolMessage" => ToolMessageAsMessage,
        "UserMessage" => UserMessageAsMessage,
    };

    // A new upstream fixture directory must be mapped above, not skipped.
    let mut on_disk: Vec<String> = fs::read_dir(spec_dir().join("fixtures"))
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    on_disk.sort();
    let mut mapped: Vec<String> = covered.iter().map(|s| s.to_string()).collect();
    mapped.sort();
    assert_eq!(on_disk, mapped);
}

/// `ToolMessage` and `UserMessage` fixtures carry their `role`, which the
/// `Message` union owns; parse them through it.
#[derive(serde::Deserialize, Serialize)]
#[serde(transparent)]
struct ToolMessageAsMessage(#[serde(deserialize_with = "tool_only")] Message);

#[derive(serde::Deserialize, Serialize)]
#[serde(transparent)]
struct UserMessageAsMessage(#[serde(deserialize_with = "user_only")] Message);

fn tool_only<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Message, D::Error> {
    match <Message as serde::Deserialize>::deserialize(d)? {
        m @ Message::Tool(_) => Ok(m),
        _ => Err(serde::de::Error::custom("not a tool message")),
    }
}

fn user_only<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Message, D::Error> {
    match <Message as serde::Deserialize>::deserialize(d)? {
        m @ Message::User(_) => Ok(m),
        _ => Err(serde::de::Error::custom("not a user message")),
    }
}

/// What everruns emits for the 1.0 features validates against the schema.
#[test]
fn emitted_1_0_shapes_validate() {
    let events = vec![
        Event::RunStarted(RunStartedEvent::new("t", "r").with_protocol_version()),
        Event::ReasoningStart(ReasoningSpanEvent::new("rs-1")),
        Event::ReasoningMessageStart(ReasoningMessageStartEvent::new("rm-1")),
        Event::ReasoningMessageContent(ReasoningMessageContentEvent::new("rm-1", "hmm")),
        Event::ReasoningMessageEnd(ReasoningMessageEndEvent::new("rm-1")),
        Event::ReasoningEnd(ReasoningSpanEvent::new("rs-1")),
        Event::ActivitySnapshot(ActivitySnapshotEvent {
            message_id: "a-1".into(),
            activity_type: "everruns.tool".into(),
            content: json!({ "text": "Working" }).as_object().unwrap().clone(),
            ..Default::default()
        }),
        Event::ToolCallStart(ToolCallStartEvent::new("call-1", "set_theme")),
        Event::ToolCallArgs(ToolCallArgsEvent::new("call-1", "{}")),
        Event::ToolCallEnd(ToolCallEndEvent::new("call-1")),
        Event::TextMessageStart(TextMessageStartEvent::assistant("m-1")),
        Event::TextMessageContent(TextMessageContentEvent::new("m-1", "hi")),
        Event::TextMessageEnd(TextMessageEndEvent::new("m-1")),
        Event::SubagentStarted(SubagentStartedEvent {
            subagent_run_id: "sa-1".into(),
            name: "Researcher".into(),
            ..Default::default()
        }),
        Event::SubagentFinished(SubagentFinishedEvent {
            subagent_run_id: "sa-1".into(),
            outcome: Some(SubagentFinishedOutcome::Success),
            ..Default::default()
        }),
        Event::RunFinished(RunFinishedEvent {
            usage: Some(vec![TokenUsage {
                provider: Some("openai".into()),
                model: Some("gpt-5".into()),
                input_tokens: Some(10),
                output_tokens: Some(5),
                total_tokens: Some(15),
                ..Default::default()
            }]),
            ..RunFinishedEvent::new("t", "r").with_outcome(RunFinishedOutcome::Success {
                pending_tool_call_ids: Some(vec!["call-1".into()]),
            })
        }),
        Event::RunFinished(RunFinishedEvent::new("t", "r").with_outcome(
            RunFinishedOutcome::Interrupt {
                interrupts: vec![Interrupt {
                    message: Some("Approve?".into()),
                    tool_call_id: Some("call-2".into()),
                    ..Interrupt::new("int-1", "tool_approval")
                }],
            },
        )),
        Event::RunFinished(
            RunFinishedEvent::new("t", "r").with_outcome(RunFinishedOutcome::Cancelled),
        ),
        Event::RunError(RunErrorEvent::new("Something went wrong").with_code("internal_error")),
    ];
    for event in events {
        let value = serde_json::to_value(&event).unwrap();
        assert_schema_valid("Event", &value);
        assert!(
            !value.to_string().contains("null"),
            "null on the wire: {value}"
        );
    }
}

#[test]
fn historical_nulls_read_as_absent() {
    let input: RunAgentInput = serde_json::from_value(json!({
        "threadId": "t",
        "runId": "r",
        "messages": [{ "id": "u1", "role": "user", "content": "hi" }],
        "tools": [{ "name": "x", "description": "d", "parameters": null }],
        "forwardedProps": null,
        "state": null,
    }))
    .unwrap();
    assert_eq!(input.forwarded_props, None);
    assert_eq!(input.state, None);
    assert_eq!(input.tools[0].parameters, None);
    assert!(input.context.is_empty());
    assert!(input.resume.is_empty());
    assert_eq!(input.protocol_version, None);
}

#[test]
fn unknown_fields_are_ignored() {
    let event: Event = serde_json::from_value(json!({
        "type": "TEXT_MESSAGE_CONTENT",
        "messageId": "m",
        "delta": "x",
        "somethingNew": 1,
    }))
    .unwrap();
    assert_eq!(
        serde_json::to_value(&event).unwrap(),
        json!({ "type": "TEXT_MESSAGE_CONTENT", "messageId": "m", "delta": "x" })
    );
}
