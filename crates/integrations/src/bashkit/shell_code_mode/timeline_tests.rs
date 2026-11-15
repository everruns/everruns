//! Every `tools` call a script makes lands in the session timeline under the
//! `bash` call it ran in.

use super::*;
use everruns_contracts::runtime::events::{EventContext, EventRequest};
use everruns_contracts::runtime::{Event, EventEmitter};

#[derive(Default)]
struct Events(Mutex<Vec<Value>>);

#[async_trait]
impl EventEmitter for Events {
    async fn emit(&self, request: EventRequest) -> everruns_contracts::error::Result<Event> {
        self.0
            .lock()
            .unwrap()
            .push(serde_json::to_value(&request).unwrap());
        Ok(request.into_event(everruns_contracts::typed_id::EventId::new(), 1))
    }
}

#[tokio::test]
async fn each_call_is_recorded_under_the_shell_call() {
    let policy = Arc::new(Policy {
        approval_for: Some("mcp_github__get_issue"),
        ..Policy::default()
    });
    let mut context = context(Some(policy), true);
    let events = Arc::new(Events::default());
    context.event_emitter = Some(events.clone());
    context.event_context = Some(EventContext::empty());

    run(
        r#"tools web-fetch repo=a/b > /dev/null
tools read-notes > /dev/null
tools github get-issue number=7"#,
        &context,
    )
    .await;

    let recorded: Vec<Value> = events
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|event| event["type"] == "tool.nested_call")
        .map(|event| event["data"].clone())
        .collect();
    let summary: Vec<(&str, &str, &str)> = recorded
        .iter()
        .map(|data| {
            (
                data["command"].as_str().unwrap(),
                data["status"].as_str().unwrap(),
                data["parent_tool_call_id"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("tools web-fetch", "completed", "call_outer"),
            ("tools read-notes", "completed", "call_outer"),
            ("tools github get-issue", "needs_approval", "call_outer"),
        ],
        "{recorded:?}"
    );
    assert_eq!(recorded[0]["input_preview"], r#"{"repo":"a/b"}"#);
    assert!(
        recorded[0]["duration_ms"].is_u64(),
        "a call that ran is timed"
    );
    assert!(
        recorded[2].get("duration_ms").is_none(),
        "a held call never ran"
    );
}
