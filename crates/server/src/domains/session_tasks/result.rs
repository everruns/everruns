// Structured task result retrieval (EVE-678 → EVE-728).
//
// A task declared with a `result_schema` records a deterministic, schema-valid
// `result.json` when its child calls `report_result` (see EVE-678,
// `everruns_platform::capabilities::subagents::ReportResultTool`). The validated
// object is written to the owning session's virtual filesystem at
// `/.tasks/{task_id}/result.json` and the owning `session_tasks` row gets a
// `result_path` pointer.
//
// The A2A and MCP task surfaces both expose a session as a "task" handle, so
// this helper resolves that structured result for a session id and hands it to
// each surface (A2A `tasks/get` artifact, MCP Tasks `tasks/get` result). See
// `knowledge/integrations/a2a-channel.md` / `knowledge/integrations/mcp.md`.

use everruns_core::SessionId;
use everruns_core::session_task::{SessionTask, TASK_KIND_SUBAGENT, task_result_path};
use serde_json::Value;

use crate::storage::StorageBackend;

/// Read the deterministic structured result (`result.json`) reported for a
/// session, if any.
///
/// Returns `Ok(None)` when the session has no owned task that reported a
/// structured result (the common case — a plain agent turn produces
/// last-message text, not a schema-bound result).
///
/// When a session owns more than one result-bearing task, the most recently
/// updated one wins. That keeps the contract deterministic for the typical
/// single-result task while still surfacing the freshest result if an app
/// re-runs the work.
///
/// # Tenant isolation
///
/// Every read is scoped by `org_id`. The session is loaded via
/// `get_session(org_id, ..)`, which both enforces the tenant boundary and
/// yields the `workspace_id` that owns the VFS where `report_result` wrote the
/// file — so a caller in one org can never read another org's result file even
/// if it guesses a session id.
pub async fn read_structured_task_result(
    db: &StorageBackend,
    org_id: i64,
    session_id: SessionId,
) -> anyhow::Result<Option<Value>> {
    // Org-scoped session load. Also yields the workspace id backing the VFS:
    // `report_result` writes keyed by the owning session's workspace, so we
    // read it back with the same key.
    let Some(session) = db.get_session(org_id, session_id).await? else {
        return Ok(None);
    };

    let rows = db.list_session_tasks(session_id, None, None).await?;
    let mut best: Option<SessionTask> = None;
    for row in &rows {
        let task = row.to_task()?;
        if !is_structured_result_task(&task) {
            continue;
        }
        match &best {
            Some(current) if current.updated_at >= task.updated_at => {}
            _ => best = Some(task),
        }
    }
    let Some(result_path) = best.and_then(|task| task.result_path) else {
        return Ok(None);
    };

    // The session VFS is keyed by the workspace id (the file was written with
    // the owning session's `workspace_id`); read the file back with that key.
    let Some(file) = db
        .get_session_file(session.workspace_id, &result_path)
        .await?
    else {
        return Ok(None);
    };
    let Some(bytes) = file.content else {
        return Ok(None);
    };
    let value = serde_json::from_slice::<Value>(&bytes)?;
    Ok(Some(value))
}

fn is_structured_result_task(task: &SessionTask) -> bool {
    task.kind == TASK_KIND_SUBAGENT
        && task.spec.get("result_schema").is_some_and(Value::is_object)
        && task
            .result_path
            .as_deref()
            .is_some_and(|path| path == task_result_path(&task.id))
}
