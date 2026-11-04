// Dataset cases as Mira samples (knowledge/evaluation/external-results-publishing.md).
//
// Decision: a dataset exports as Mira `Sample` rows and imports from them, so
// one file runs in Mira and in Everruns. A sample carries the scripted user
// turns (`input`), tags and a free-form `metadata` map. Everything Everruns
// needs beyond that (scorers, post messages, simulated user, tool fixtures,
// limits) rides under `metadata.everruns`, which Mira keeps and ignores, so an
// export imports back unchanged.
//
// Decision: a sample written for Mira carries no `metadata.everruns`. Its
// `expected` text becomes a `contains` scorer, the nearest Everruns check to
// Mira's answer comparison; any other `expected` value imports unscored.
// `attachments` and `files` have no Everruns case field: the sample still
// imports, and the report names what was left behind.
//
// Decision: cases are matched by name (the sample id), like ATIF import, so a
// re-import converges instead of duplicating.

use super::queries as q;
use super::record::{EvalCase, EvalInputMessage, EvalTarget, Scorer};
use super::types::{CreateEvalCaseRequest, UpdateEvalCaseRequest};
use crate::domains::common::*;
use crate::domains::evals::multistep::{SimulatedUser, ToolFixture};
use crate::domains::evals::record::ArtifactSpec;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use utoipa::ToSchema;

/// Most samples one import takes.
const MAX_SAMPLES: usize = 500;

fn require_evals_enabled(ctx: &Ctx) -> Result<(), CommandError> {
    if ctx.feature_flags.evals {
        Ok(())
    } else {
        Err(CommandError::feature_not_enabled("evals"))
    }
}

/// One Mira dataset row (`mira::Sample`).
#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
pub struct EvalSample {
    /// Stable id within the dataset: the case name.
    #[schema(example = "refund-over-limit")]
    pub id: String,
    /// User turns, sent in order.
    #[schema(example = json!(["Refund $450 for order 1182"]))]
    pub input: Vec<String>,
    /// Reference answer. Text becomes a `contains` scorer on import.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub expected: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Non-text input. Not imported.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schema(value_type = Vec<Object>)]
    pub attachments: Vec<Value>,
    /// Files to seed into the workspace. Not imported.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    #[schema(value_type = Object)]
    pub files: Map<String, Value>,
    /// Free-form metadata. `everruns` holds the case settings Mira has no
    /// field for.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    #[schema(value_type = Object)]
    pub metadata: Map<String, Value>,
}

/// The Everruns-only part of a case, under `metadata.everruns`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct CaseSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    case_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scorers: Option<Vec<Scorer>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    post: Option<Vec<EvalInputMessage>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target: Option<EvalTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    artifacts: Option<Vec<ArtifactSpec>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_turns: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_seconds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    simulated_user: Option<SimulatedUser>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_fixtures: Option<Vec<ToolFixture>>,
}

/// A case as a Mira sample.
pub fn case_to_sample(case: &EvalCase) -> EvalSample {
    let settings = CaseSettings {
        case_id: Some(case.public_id.to_string()),
        description: case.description.clone(),
        scorers: (!case.scorers.is_empty()).then(|| case.scorers.clone()),
        post: case.post.clone(),
        target: case.target.clone(),
        artifacts: case.artifacts.clone(),
        max_turns: case.max_turns,
        timeout_seconds: case.timeout_seconds,
        simulated_user: case.simulated_user.clone(),
        tool_fixtures: case.tool_fixtures.clone(),
    };
    let mut metadata = Map::new();
    if let Ok(value) = serde_json::to_value(&settings) {
        metadata.insert("everruns".into(), value);
    }
    EvalSample {
        id: case.name.clone(),
        input: case
            .conversation
            .iter()
            .map(|m| m.content.clone())
            .collect(),
        tags: case.tags.clone(),
        metadata,
        ..Default::default()
    }
}

/// What a sample becomes, and what it had that a case cannot hold.
#[derive(Debug)]
pub struct SampleCase {
    pub create: CreateEvalCaseRequest,
    pub left_behind: Vec<&'static str>,
}

/// A Mira sample as a case.
pub fn sample_to_case(sample: EvalSample) -> Result<SampleCase, String> {
    let name = sample.id.trim().to_string();
    if name.is_empty() {
        return Err("a sample has no id".into());
    }
    let settings: CaseSettings = match sample.metadata.get("everruns") {
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|e| format!("sample '{name}': metadata.everruns: {e}"))?,
        None => CaseSettings::default(),
    };
    let scorers = match (settings.scorers, &sample.expected) {
        (Some(scorers), _) => scorers,
        (None, Some(Value::String(text))) if !text.trim().is_empty() => {
            vec![Scorer::Contains {
                text: text.clone(),
                weight: 1.0,
            }]
        }
        (None, _) => Vec::new(),
    };
    let mut left_behind = Vec::new();
    if !sample.attachments.is_empty() {
        left_behind.push("attachments");
    }
    if !sample.files.is_empty() {
        left_behind.push("files");
    }
    if sample.expected.is_some() && scorers.is_empty() {
        left_behind.push("expected");
    }
    Ok(SampleCase {
        create: CreateEvalCaseRequest {
            name,
            description: settings.description,
            target: settings.target,
            tags: (!sample.tags.is_empty()).then_some(sample.tags),
            conversation: sample
                .input
                .into_iter()
                .map(|content| EvalInputMessage { content })
                .collect(),
            post: settings.post,
            artifacts: settings.artifacts,
            scorers,
            max_turns: settings.max_turns,
            timeout_seconds: settings.timeout_seconds,
            position: None,
            simulated_user: settings.simulated_user,
            tool_fixtures: settings.tool_fixtures,
        },
        left_behind,
    })
}

fn update_from(create: CreateEvalCaseRequest) -> UpdateEvalCaseRequest {
    UpdateEvalCaseRequest {
        name: None,
        description: create.description,
        target: create.target,
        tags: create.tags,
        conversation: Some(create.conversation),
        post: create.post,
        artifacts: create.artifacts,
        scorers: Some(create.scorers),
        max_turns: create.max_turns,
        timeout_seconds: create.timeout_seconds,
        position: None,
        simulated_user: Some(create.simulated_user),
        tool_fixtures: Some(create.tool_fixtures),
    }
}

// ============================================
// Commands
// ============================================

/// A dataset as Mira samples.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EvalSampleList {
    pub samples: Vec<EvalSample>,
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct ExportEvalSamples {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
}

#[command(
    name = "export_eval_samples",
    category = "evals",
    summary = "Export a dataset as Mira samples",
    description = "Export an eval's cases as Mira samples, one per case. Settings Mira has no field for ride under metadata.everruns, so the export imports back unchanged.",
    method = "GET",
    cli = CliRoute::new(&["evals", "samples"], "export").with_examples(&[CliExample::new("Save a dataset as a Mira JSONL file", "everruns evals samples export --eval-id eval_01h9 | jq -c '.samples[]' > cases.jsonl")]),
    path = "/v1/evals/{eval_id}/samples",
    policy = crate::domains::evals::EVAL_VIEW,
    http = plain,
)]
impl Command for ExportEvalSamples {
    type Output = EvalSampleList;

    async fn execute(self, ctx: &Ctx) -> Result<EvalSampleList, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?;
        let cases = q::service(ctx)
            .list_cases(&ctx.caller, &eval_id.to_string())
            .await
            .map_err(classify_anyhow)?;
        Ok(EvalSampleList {
            samples: cases.iter().map(case_to_sample).collect(),
        })
    }
}

/// What a sample import did.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EvalSampleImportReport {
    /// Cases created.
    #[schema(example = 3)]
    pub created: u64,
    /// Existing cases updated, matched by name.
    #[schema(example = 1)]
    pub updated: u64,
    /// Public ids of the affected cases, in import order.
    pub case_ids: Vec<String>,
    /// Sample fields a case cannot hold, one line per sample that had them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schema(example = json!(["refund-with-receipt: attachments"]))]
    pub left_behind: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct ImportEvalSamples {
    /// Eval's prefixed public identifier.
    pub eval_id: String,
    /// Mira samples. Each becomes the case named by its id.
    pub samples: Vec<EvalSample>,
}

#[command(
    name = "import_eval_samples",
    category = "evals",
    summary = "Import Mira samples as cases",
    description = "Create or update an eval's cases from Mira samples, matched by name. A sample's expected text becomes a contains scorer unless metadata.everruns names the scorers.",
    method = "POST",
    cli = CliRoute::new(&["evals", "samples"], "import").with_examples(&[CliExample::new("Load a Mira JSONL dataset into an eval", "everruns evals samples import --eval-id eval_01h9 --samples \"$(jq -s . cases.jsonl)\" --reason 'Share the Mira regression set'")]),
    path = "/v1/evals/{eval_id}/samples",
    policy = crate::domains::evals::EVAL_MANAGE,
    http = plain,
)]
impl Command for ImportEvalSamples {
    type Output = EvalSampleImportReport;

    async fn execute(self, ctx: &Ctx) -> Result<EvalSampleImportReport, CommandError> {
        require_evals_enabled(ctx)?;
        let eval_id = q::parse_eval_id(&self.eval_id)?.to_string();
        if self.samples.is_empty() {
            return Err(CommandError::bad_request(
                "an import needs at least one sample",
            ));
        }
        // THREAT[TM-DOS-025]: bound the fan-out before any case is written.
        if self.samples.len() > MAX_SAMPLES {
            return Err(CommandError::bad_request(format!(
                "an import takes at most {MAX_SAMPLES} samples"
            )));
        }
        // Convert everything first, so one bad sample writes nothing.
        let converted = self
            .samples
            .into_iter()
            .map(sample_to_case)
            .collect::<Result<Vec<_>, _>>()
            .map_err(CommandError::bad_request)?;

        let service = q::service(ctx);
        let mut by_name: HashMap<String, String> = service
            .list_cases(&ctx.caller, &eval_id)
            .await
            .map_err(classify_anyhow)?
            .into_iter()
            .map(|c| (c.name.clone(), c.public_id.to_string()))
            .collect();
        let mut report = EvalSampleImportReport {
            created: 0,
            updated: 0,
            case_ids: Vec::with_capacity(converted.len()),
            left_behind: Vec::new(),
        };
        for SampleCase {
            create,
            left_behind,
        } in converted
        {
            let name = create.name.clone();
            if !left_behind.is_empty() {
                report
                    .left_behind
                    .push(format!("{name}: {}", left_behind.join(", ")));
            }
            let case_id = match by_name.get(&name).cloned() {
                Some(case_id) => {
                    service
                        .update_case(&ctx.caller, &eval_id, &case_id, update_from(create))
                        .await
                        .map_err(classify_anyhow)?
                        .ok_or_else(|| CommandError::not_found("EvalCase"))?;
                    report.updated += 1;
                    case_id
                }
                None => {
                    let case = service
                        .create_case(&ctx.caller, &eval_id, create)
                        .await
                        .map_err(classify_anyhow)?;
                    report.created += 1;
                    let case_id = case.public_id.to_string();
                    by_name.insert(name, case_id.clone());
                    case_id
                }
            };
            report.case_ids.push(case_id);
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_mira_sample_imports_with_its_expected_text_as_a_check() {
        let sample: EvalSample = serde_json::from_value(serde_json::json!({
            "id": "greet",
            "input": ["Say hi", "Now the answer"],
            "expected": "42",
            "tags": ["smoke"],
            "files": {"main.rs": "fn main() {}"}
        }))
        .unwrap();
        let converted = sample_to_case(sample).unwrap();
        assert_eq!(converted.create.name, "greet");
        assert_eq!(converted.create.conversation.len(), 2);
        assert_eq!(converted.create.tags, Some(vec!["smoke".to_string()]));
        assert!(matches!(
            converted.create.scorers.as_slice(),
            [Scorer::Contains { text, .. }] if text == "42"
        ));
        assert_eq!(converted.left_behind, vec!["files"]);
    }

    #[test]
    fn a_structured_expected_value_is_reported_not_guessed() {
        let sample = EvalSample {
            id: "json".into(),
            input: vec!["List three colors".into()],
            expected: Some(serde_json::json!({"colors": 3})),
            ..Default::default()
        };
        let converted = sample_to_case(sample).unwrap();
        assert!(converted.create.scorers.is_empty());
        assert_eq!(converted.left_behind, vec!["expected"]);
    }

    #[test]
    fn everruns_settings_win_over_expected_and_round_trip() {
        let sample: EvalSample = serde_json::from_value(serde_json::json!({
            "id": "refund",
            "input": ["Refund $450"],
            "expected": "refund",
            "metadata": {"everruns": {
                "description": "Over the limit",
                "scorers": [{"type": "tool_called", "tool": "handoff"}],
                "max_turns": 4
            }, "owner": "support"}
        }))
        .unwrap();
        let converted = sample_to_case(sample).unwrap();
        assert_eq!(
            converted.create.description.as_deref(),
            Some("Over the limit")
        );
        assert_eq!(converted.create.max_turns, Some(4));
        assert!(matches!(
            converted.create.scorers.as_slice(),
            [Scorer::ToolCalled { .. }]
        ));
        assert!(converted.left_behind.is_empty());
    }

    #[test]
    fn bad_samples_are_refused() {
        assert!(sample_to_case(EvalSample::default()).is_err());
        let sample: EvalSample = serde_json::from_value(serde_json::json!({
            "id": "x", "input": ["hi"], "metadata": {"everruns": {"max_turns": "many"}}
        }))
        .unwrap();
        assert!(
            sample_to_case(sample)
                .unwrap_err()
                .contains("metadata.everruns")
        );
    }
}
