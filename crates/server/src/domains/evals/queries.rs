use crate::domains::common::{CommandError, Ctx};
use crate::domains::evals::EvalService;
use everruns_platform::eval::EvalCaseResult;
use everruns_provider::typed_id::{EvalCaseId, EvalId, EvalResultId, EvalRunId};
use serde_json::{Map, Value};
use std::sync::Arc;

pub fn service(ctx: &Ctx) -> Arc<EvalService> {
    ctx.eval_service
        .clone()
        .unwrap_or_else(|| Arc::new(EvalService::new(ctx.db.clone())))
}

pub fn parse_eval_id(eval_id: &str) -> Result<EvalId, CommandError> {
    eval_id
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid eval ID: {e}")))
}

pub fn parse_case_id(case_id: &str) -> Result<EvalCaseId, CommandError> {
    case_id
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid case ID: {e}")))
}

pub fn parse_run_id(run_id: &str) -> Result<EvalRunId, CommandError> {
    run_id
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid run ID: {e}")))
}

pub fn parse_result_id(result_id: &str) -> Result<EvalResultId, CommandError> {
    result_id
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid result ID: {e}")))
}

pub fn run_artifact_export_value(result: &EvalCaseResult) -> Value {
    let mut export = Map::new();
    export.insert(
        "instance_id".to_string(),
        Value::String(
            result
                .case_name
                .clone()
                .unwrap_or_else(|| result.eval_case_id.to_string()),
        ),
    );

    if let Some(artifacts) = &result.artifacts {
        for (name, content) in artifacts {
            let key = if name == "patch" {
                "model_patch"
            } else {
                name.as_str()
            };
            if export.contains_key(key) {
                tracing::warn!(
                    eval_case_id = %result.eval_case_id,
                    artifact_name = %name,
                    export_key = %key,
                    "Skipping colliding eval artifact export field"
                );
                continue;
            }
            export.insert(key.to_string(), Value::String(content.clone()));
        }
    }

    Value::Object(export)
}
