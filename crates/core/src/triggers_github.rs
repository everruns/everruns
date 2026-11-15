//! GitHub event normalization, shared by Framework and Server.
use serde_json::{Value, json};
/// A signed delivery, as received from GitHub.
#[derive(Debug, Clone)]
pub struct GitHubDelivery {
    /// `X-GitHub-Event`.
    pub event: String,
    /// `X-GitHub-Delivery`; the dedupe key.
    pub delivery_id: String,
    /// Parsed JSON body.
    pub payload: Value,
}

/// Normalize repository, item, actor and CI facts from an authenticated delivery.
pub fn github_context(delivery: &GitHubDelivery) -> Value {
    let payload = &delivery.payload;
    let action = payload.get("action").and_then(Value::as_str);
    let item = payload
        .get("pull_request")
        .or_else(|| payload.get("issue"))
        .filter(|item| item.is_object());
    let repository = payload
        .pointer("/repository/full_name")
        .and_then(Value::as_str);
    json!({
        "event": delivery.event,
        "action": action,
        "event_action": event_key(&delivery.event, action),
        "delivery": delivery.delivery_id,
        "repository": repository,
        "number": item.and_then(|item| item.get("number")).or_else(|| payload.get("number")),
        "title": item.and_then(|item| item.get("title")),
        "url": item.and_then(|item| item.get("html_url")),
        "is_pull_request": payload.get("pull_request").is_some()
            || payload.pointer("/issue/pull_request").is_some(),
        "sender": payload.pointer("/sender/login"),
        "ci": ci_context(delivery),
    })
}

/// CI facts for `workflow_run` and `check_suite` deliveries; null otherwise.
fn ci_context(delivery: &GitHubDelivery) -> Value {
    let payload = &delivery.payload;
    let (run, name, url) = match delivery.event.as_str() {
        "workflow_run" => {
            let Some(run) = payload.get("workflow_run").filter(|run| run.is_object()) else {
                return Value::Null;
            };
            (run, run.get("name").cloned(), run.get("html_url").cloned())
        }
        "check_suite" => {
            let Some(suite) = payload.get("check_suite").filter(|suite| suite.is_object()) else {
                return Value::Null;
            };
            let url = payload
                .pointer("/repository/html_url")
                .and_then(Value::as_str)
                .zip(suite.get("head_sha").and_then(Value::as_str))
                .map(|(repo, sha)| Value::String(format!("{repo}/commit/{sha}/checks")));
            (suite, suite.pointer("/app/name").cloned(), url)
        }
        _ => return Value::Null,
    };
    let conclusion = run.get("conclusion").and_then(Value::as_str);
    json!({
        "workflow": name,
        "branch": run.get("head_branch"),
        "head_sha": run.get("head_sha"),
        "status": run.get("status"),
        "conclusion": conclusion,
        "outcome": ci_outcome(conclusion),
        "url": url,
        "run_number": run.get("run_number"),
        "actor": run.pointer("/actor/login").or_else(|| payload.pointer("/sender/login")),
    })
}

/// `success` or `failure`; null for results that say nothing about health
/// (cancelled, skipped, neutral, stale, or not finished), so a cancelled run
/// never reads as a recovery.
fn ci_outcome(conclusion: Option<&str>) -> Option<&'static str> {
    match conclusion? {
        "success" => Some("success"),
        "failure" | "timed_out" | "action_required" | "startup_failure" => Some("failure"),
        _ => None,
    }
}

/// Join a GitHub event and its optional action into one subscription key.
pub fn event_key(event: &str, action: Option<&str>) -> String {
    match action {
        Some(action) => format!("{event}.{action}"),
        None => event.to_string(),
    }
}

/// `owner/repo#12` for pull request and issue events, `owner/repo@branch:workflow`
/// for CI results, `owner/repo` otherwise.
pub fn subject(github: &Value) -> Option<String> {
    let repository = github.get("repository").and_then(Value::as_str)?;
    let ci = github.get("ci").filter(|ci| ci.is_object());
    if let Some(ci) = ci {
        let branch = ci.get("branch").and_then(Value::as_str).unwrap_or("");
        let workflow = ci.get("workflow").and_then(Value::as_str).unwrap_or("");
        return Some(format!("{repository}@{branch}:{workflow}"));
    }
    Some(match github.get("number").and_then(Value::as_i64) {
        Some(number) => format!("{repository}#{number}"),
        None => repository.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::ci_outcome;

    #[test]
    fn ci_outcome_ignores_results_that_say_nothing_about_health() {
        assert_eq!(ci_outcome(Some("success")), Some("success"));
        for failed in ["failure", "timed_out", "action_required", "startup_failure"] {
            assert_eq!(ci_outcome(Some(failed)), Some("failure"), "{failed}");
        }
        for quiet in ["cancelled", "skipped", "neutral", "stale"] {
            assert_eq!(ci_outcome(Some(quiet)), None, "{quiet}");
        }
        assert_eq!(ci_outcome(None), None);
    }
}
